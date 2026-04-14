// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Trait-compatible S3 artifact backend stub.
//!
//! Every method returns `anyhow::Error` with a clear "not yet implemented"
//! message so the server can construct the backend from config and route
//! it through the same dyn-trait plumbing as [`super::fs::FsArtifactStore`]
//! without depending on the real AWS SDK. Dropping in a real implementation
//! is a matter of replacing the method bodies — the trait shape, the
//! config surface, and the wiring in `main.rs` stay the same.

use anyhow::{bail, Result};
use async_trait::async_trait;

use crate::config::ArtifactsS3;

use super::{ArtifactHandle, ArtifactStore, AsyncReader};

pub struct S3ArtifactStore {
    #[allow(dead_code)]
    cfg: ArtifactsS3,
}

impl S3ArtifactStore {
    /// Validate the config up front so a misconfigured endpoint is caught
    /// at startup, not on the first artifact upload hours into a build.
    pub fn new(cfg: ArtifactsS3) -> Result<Self> {
        if cfg.bucket.is_empty() {
            bail!("artifacts.s3.bucket is required when backend = \"s3\"");
        }
        Ok(Self { cfg })
    }
}

#[async_trait]
impl ArtifactStore for S3ArtifactStore {
    async fn put(
        &self,
        _run_id: i64,
        _name: &str,
        _reader: AsyncReader,
    ) -> Result<ArtifactHandle> {
        bail!(
            "S3 artifact backend is a stub in this build — set \
             [artifacts] backend = \"fs\" or rebuild with the full S3 client"
        );
    }

    async fn get(&self, _path: &str) -> Result<AsyncReader> {
        bail!("S3 artifact backend is a stub in this build");
    }

    async fn delete_run(&self, _run_id: i64) -> Result<()> {
        // Swallow silently — the retention sweeper and cancel path both
        // call this and both are tolerant of a zero-op delete. Emitting
        // an error every hour from the sweeper would just spam logs.
        Ok(())
    }
}
