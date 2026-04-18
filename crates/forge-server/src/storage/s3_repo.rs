// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! `RepoStorageBackend` implementation that puts the LIVE object store
//! in S3 while keeping staging on the local filesystem (Phase 3b.4).
//!
//! ## Why hybrid
//!
//! The Phase-1 push path appends chunks into a per-session staging
//! file as they arrive from the gRPC stream — `StagingStore::append`
//! is O(n) in calls because each one translates to a single
//! `OpenOptions::append` + `write_all`. S3 has no native append
//! primitive; emulating one would require either buffering every
//! chunk of an in-flight push in memory (blows up at the 16-GiB
//! `[limits] max_object_size` default) or rotating a multipart
//! upload for every object, which chews through concurrent-upload
//! quotas and doubles every dedup probe into a `head_object` +
//! `list_parts` round-trip.
//!
//! Staging on local disk sidesteps all of that. `promote_into`
//! at commit time reads each staged file and `put_raw`-uploads it
//! to S3 — slower than the FS→FS rename but only ever touched on
//! the successful commit path, so the hot upload loop stays on the
//! fastest medium available (local SSD).
//!
//! ## Thread-safety
//!
//! `S3ObjectBackend` wraps an `aws_sdk_s3::Client` which is internally
//! arc-backed. `scoped(prefix)` clones the client (cheap) and the
//! `String` prefix (cheap) — repeat calls to
//! `repo_object_backend(repo)` don't open a new TCP / HTTPS session.
//!
//! ## Rename / delete (Phase 3b.5)
//!
//! S3 lacks an atomic "move prefix" primitive. `rename_repo` and
//! `delete_repo` enqueue a durable work item in `pending_repo_ops`
//! (via [`MetadataBackend::enqueue_repo_op`]) and return immediately;
//! a background drain task walks the bucket keyspace with CopyObject
//! + batched DeleteObjects out-of-band. A server restart mid-drain
//! resumes the op from the DB row — no lost work, no stranded keys.

#![cfg(feature = "s3-objects")]

use std::io;
use std::sync::Arc;

use forge_core::store::backend::ObjectBackend;

use crate::storage::backend::MetadataBackend;
use crate::storage::fs::FsStorage;
use crate::storage::repo_backend::{RepoStorageBackend, StagingBackend};
use crate::storage::s3_objects::S3ObjectBackend;

/// Live-in-S3, stage-on-FS repo storage. Construct at startup from
/// the `[objects.s3]` config block; hand to `ForgeGrpcService` as
/// the `storage` field.
pub struct S3RepoStorage {
    /// The base S3 backend. Carries the bucket + credentials + any
    /// global prefix operators configured; `scoped()` adds the per-
    /// repo `{repo}/objects/` tail on each `repo_object_backend`
    /// call.
    pub base: Arc<S3ObjectBackend>,
    /// Staging lives on local disk. Phase 1's push path
    /// (`ensure_shard_dirs` + `append` + `promote_into`) stays
    /// unchanged; the promote walk reads each staged file and uploads
    /// via the S3 live backend's `put_raw`.
    pub fs: Arc<FsStorage>,
    /// Durable drain queue for rename/delete. When `Some`, the
    /// lifecycle RPCs enqueue a work item and return immediately;
    /// when `None` (unit tests that don't care about S3 cleanup),
    /// they log a warning and skip the S3 side. Production construction
    /// always sets this.
    pub queue: Option<Arc<dyn MetadataBackend>>,
}

impl S3RepoStorage {
    /// Construct without a drain queue. Suitable only for code paths
    /// that never rename/delete repos (admin tooling probes, tests).
    /// Production code must go through [`Self::with_queue`].
    pub fn new(base: Arc<S3ObjectBackend>, fs: Arc<FsStorage>) -> Self {
        Self {
            base,
            fs,
            queue: None,
        }
    }

    /// Construct with a drain queue. `serve_inner` calls this so
    /// `rename_repo` / `delete_repo` durably enqueue their S3-side
    /// work instead of silently warning.
    pub fn with_queue(
        base: Arc<S3ObjectBackend>,
        fs: Arc<FsStorage>,
        queue: Arc<dyn MetadataBackend>,
    ) -> Self {
        Self {
            base,
            fs,
            queue: Some(queue),
        }
    }
}

impl RepoStorageBackend for S3RepoStorage {
    fn repo_object_backend(&self, repo: &str) -> Arc<dyn ObjectBackend> {
        // `{repo}/objects/` matches the FS layout the CLI speaks,
        // so a MinIO shim could serve either tree without rewriting
        // keys.
        let scoped = self.base.scoped(&format!("{repo}/objects"));
        Arc::new(scoped)
    }

    fn session_staging(&self, repo: &str, session_id: &str) -> Box<dyn StagingBackend> {
        // Staging defers to FsStorage unchanged — the cross-backend
        // `StagingBackend::promote_into` path in `repo_backend.rs`
        // handles FS-staged → S3-live via `live.put_raw`.
        self.fs.session_staging(repo, session_id)
    }

    fn purge_session_staging(&self, repo: &str, session_id: &str) -> io::Result<()> {
        // Staging is local — no S3 round-trip.
        FsStorage::purge_session_staging(&self.fs, repo, session_id)
    }

    fn rename_repo(&self, old_name: &str, new_name: &str) -> io::Result<()> {
        // FS side always renames local staging. S3 side goes on the
        // durable drain queue — rename of a 1 TB repo can take many
        // minutes of CopyObject+DeleteObject work, we're not holding
        // the RPC open for that.
        FsStorage::rename_repo(&self.fs, old_name, new_name)?;
        if let Some(q) = self.queue.as_ref() {
            q.enqueue_repo_op("rename", old_name, Some(new_name))
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
            tracing::info!(
                old = old_name,
                new = new_name,
                "S3RepoStorage::rename_repo queued S3 prefix move; drain will relocate live objects"
            );
        } else {
            tracing::warn!(
                old = old_name,
                new = new_name,
                "S3RepoStorage::rename_repo has no drain queue — S3 prefix \
                 not moved. Wire with_queue() in production construction."
            );
        }
        Ok(())
    }

    fn delete_repo(&self, name: &str) -> io::Result<()> {
        FsStorage::delete_repo(&self.fs, name)?;
        if let Some(q) = self.queue.as_ref() {
            q.enqueue_repo_op("delete", name, None)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
            tracing::info!(
                repo = name,
                "S3RepoStorage::delete_repo queued S3 prefix wipe; drain will reclaim bucket keys"
            );
        } else {
            tracing::warn!(
                repo = name,
                "S3RepoStorage::delete_repo has no drain queue — S3 prefix \
                 not deleted. Wire with_queue() in production construction."
            );
        }
        Ok(())
    }

    // `repo_local_path` keeps the default `None` from the trait — S3
    // repos have no local root. Callers that rely on a local path
    // (the repo-stats walker in grpc.rs) already gate on that Option
    // after Phase 3b.3.
}
