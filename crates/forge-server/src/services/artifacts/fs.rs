// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Local-filesystem artifact store. Mirrors the pre-Phase-1 directory
//! layout: `<root>/<run_id>/<name>` with a single file per artifact. Upload
//! streams to a `.part` sibling and renames on success — partial writes
//! never show up as finished artifacts.

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs as tfs;
use tokio::io::AsyncWriteExt;

use super::{ArtifactHandle, ArtifactStore, AsyncReader};

pub struct FsArtifactStore {
    root: PathBuf,
}

impl FsArtifactStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn artifact_path(&self, run_id: i64, name: &str) -> PathBuf {
        self.root.join(run_id.to_string()).join(name)
    }

    fn run_dir(&self, run_id: i64) -> PathBuf {
        self.root.join(run_id.to_string())
    }
}

#[async_trait]
impl ArtifactStore for FsArtifactStore {
    async fn put(
        &self,
        run_id: i64,
        name: &str,
        mut reader: AsyncReader,
    ) -> Result<ArtifactHandle> {
        let dest = self.artifact_path(run_id, name);
        if let Some(parent) = dest.parent() {
            tfs::create_dir_all(parent)
                .await
                .with_context(|| format!("create artifact dir {}", parent.display()))?;
        }

        let tmp = dest.with_extension("part");
        let mut out = tfs::File::create(&tmp)
            .await
            .with_context(|| format!("create tmp artifact {}", tmp.display()))?;

        let mut buf = vec![0u8; 4 * 1024 * 1024];
        let mut total: i64 = 0;
        loop {
            let n = tokio::io::AsyncReadExt::read(&mut reader, &mut buf).await?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).await?;
            total += n as i64;
        }
        out.flush().await?;
        drop(out);

        tfs::rename(&tmp, &dest)
            .await
            .with_context(|| format!("finalize artifact {}", dest.display()))?;

        // Store the path relative to the root; the caller uses that as the
        // handle to round-trip back through `get`.
        let rel = dest
            .strip_prefix(&self.root)
            .unwrap_or(&dest)
            .to_string_lossy()
            .replace('\\', "/");

        Ok(ArtifactHandle {
            path: rel,
            size_bytes: total,
        })
    }

    async fn get(&self, path: &str) -> Result<AsyncReader> {
        let full = self.root.join(path);
        let file = tfs::File::open(&full)
            .await
            .with_context(|| format!("open artifact {}", full.display()))?;
        Ok(Box::pin(file))
    }

    async fn delete_run(&self, run_id: i64) -> Result<()> {
        let dir = self.run_dir(run_id);
        if !Path::new(&dir).exists() {
            return Ok(());
        }
        tfs::remove_dir_all(&dir)
            .await
            .with_context(|| format!("remove artifact dir {}", dir.display()))?;
        Ok(())
    }
}
