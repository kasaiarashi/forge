// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Artifact storage backend. Pluggable: the default is a local-filesystem
//! layout that matches the pre-Phase-1 `collect_artifact` paths; an S3
//! backend is a drop-in replacement gated behind the `s3` cargo feature.
//!
//! The trait is built around streaming [`tokio::io::AsyncRead`] so uploads
//! and downloads never materialise whole blobs in memory — critical for UE
//! builds where a single pak artifact runs into the tens of GB.

use anyhow::Result;
use async_trait::async_trait;
use std::pin::Pin;

pub mod fs;
pub mod retention;
pub mod s3;
pub mod signed_url;

/// Handle returned after a successful upload. `path` is backend-specific
/// (a relative FS path, an S3 key, …) and is stored verbatim in the
/// `artifacts` table for later retrieval.
#[derive(Debug, Clone)]
pub struct ArtifactHandle {
    pub path: String,
    pub size_bytes: i64,
}

/// `AsyncRead + Send + Unpin` trait object. Used as the reader type for both
/// uploads (caller passes in) and downloads (backend hands back).
pub type AsyncReader = Pin<Box<dyn tokio::io::AsyncRead + Send + Unpin>>;

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    /// Stream `reader` into the backend, storing under a
    /// backend-determined path keyed on `run_id` and `name`.
    async fn put(
        &self,
        run_id: i64,
        name: &str,
        reader: AsyncReader,
    ) -> Result<ArtifactHandle>;

    /// Stream an artifact back. `path` is the value previously returned by
    /// [`ArtifactStore::put`].
    async fn get(&self, path: &str) -> Result<AsyncReader>;

    /// Delete every artifact belonging to `run_id`. Idempotent — called from
    /// both the retention sweeper and the run-cancel path, where the run
    /// may have produced zero artifacts.
    async fn delete_run(&self, run_id: i64) -> Result<()>;
}
