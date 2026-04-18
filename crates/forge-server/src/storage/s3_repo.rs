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

#![cfg(feature = "s3-objects")]

use std::io;
use std::sync::Arc;

use forge_core::store::backend::ObjectBackend;

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
}

impl S3RepoStorage {
    pub fn new(base: Arc<S3ObjectBackend>, fs: Arc<FsStorage>) -> Self {
        Self { base, fs }
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
        // FS side always renames its local staging tree. S3 side
        // would need a full-prefix CopyObject + DeleteObjects pass,
        // which isn't atomic and can take minutes on large repos.
        // Surface as a warning; operators who rename repos on an S3
        // deployment should do the S3-side move manually for now.
        FsStorage::rename_repo(&self.fs, old_name, new_name)?;
        tracing::warn!(
            old = old_name,
            new = new_name,
            "S3RepoStorage::rename_repo moved the local staging tree but \
             did NOT rename the S3 prefix. Run the equivalent `aws s3 mv` \
             (or `mc mv` for MinIO) by hand."
        );
        Ok(())
    }

    fn delete_repo(&self, name: &str) -> io::Result<()> {
        // Same story as rename: FS side always cleans up; S3 side
        // would require list_objects_v2 + batched delete_objects
        // over potentially millions of keys. Punt to operator
        // tooling for 3b.4 — Phase 3b.5 wires a background drain.
        FsStorage::delete_repo(&self.fs, name)?;
        tracing::warn!(
            repo = name,
            "S3RepoStorage::delete_repo removed the local staging tree \
             but did NOT delete S3-resident live objects. Run `aws s3 rm \
             --recursive` (or `mc rm --recursive`) to reclaim the bucket \
             prefix."
        );
        Ok(())
    }

    // `repo_local_path` keeps the default `None` from the trait — S3
    // repos have no local root. Callers that rely on a local path
    // (the repo-stats walker in grpc.rs) already gate on that Option
    // after Phase 3b.3.
}
