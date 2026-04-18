// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Server-side repo-storage abstraction (Phase 3b.2).
//!
//! Forge's live object surface already factors through
//! [`forge_core::store::backend::ObjectBackend`] — that's the contract
//! both `ChunkStore` (FS) and [`crate::storage::s3_objects::S3ObjectBackend`]
//! satisfy. The *staging* surface — per-session upload directories,
//! promote-on-commit — has stayed concrete on `FsStorage` because S3
//! doesn't support append and the Phase-1 push path uses `append` to
//! land chunks one at a time.
//!
//! This module introduces the missing contract:
//!
//! - [`StagingBackend`] — the per-session write surface the gRPC
//!   push handler uses (`ensure_shard_dirs`, `put`, `append`,
//!   `file_size`, `promote`).
//! - [`RepoStorageBackend`] — per-repo dispatch that hands out a
//!   live `Arc<dyn ObjectBackend>` and a staging session on demand,
//!   plus the repo-lifecycle helpers (`rename_repo`, `delete_repo`,
//!   `purge_session_staging`) the server already calls on
//!   `FsStorage`.
//!
//! [`FsStorage`] picks up both traits via a thin delegation impl at
//! the bottom of this file — zero behaviour change.
//!
//! **gRPC wiring is deferred.** Phase 3b.3 swaps the ~20
//! `Arc<FsStorage>` sites in `services::grpc` (and the ObjectStore
//! construction at line 94) to `Arc<dyn RepoStorageBackend>` + a
//! neutral `ObjectStore` constructor. That refactor also touches
//! `forge-cli` (which accesses `ObjectStore.chunks` directly from
//! ~40 call sites); committing the trait contract first lets the
//! Phase-3b.3 change be mechanical rather than re-designing while
//! swapping. The S3-live-with-FS-staging deployment becomes real
//! once 3b.3 lands.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use forge_core::hash::ForgeHash;
use forge_core::store::backend::ObjectBackend;

use crate::storage::fs::{FsStorage, PromoteStats, StagingStore};

/// Per-upload-session write surface used by PushObjects and
/// CommitPush. Staging sits alongside the live store during the push
/// hot path; on commit, [`StagingBackend::promote_into`] is called
/// inside the same DB transaction that CAS-updates the ref.
///
/// The trait is sync because the Phase-1 push path is sync — tokio's
/// streaming-chunk handler calls these via `blocking` tasks already.
pub trait StagingBackend: Send + Sync {
    /// Pre-create the 256 shard directories so per-object writes
    /// skip `create_dir_all` on the hot path. No-op for backends
    /// whose "shards" are virtual (an object-store prefix).
    fn ensure_shard_dirs(&self) -> io::Result<()>;

    /// Single-shot write. Used when a chunk arrives `is_last =
    /// true` and fits in one message — avoids the
    /// append/open-for-write overhead.
    fn put(&self, hash: &ForgeHash, data: &[u8]) -> io::Result<()>;

    /// Append bytes to a staged object. Streaming pushes land each
    /// chunk with this call. For non-appendable backends (S3), the
    /// impl must buffer until the object is complete and translate
    /// the final write into a single put.
    fn append(&self, hash: &ForgeHash, data: &[u8]) -> io::Result<()>;

    /// On-disk size of the staged object, or `None` when absent.
    /// Used by `QueryUploadSession` to report how many bytes the
    /// server holds vs. what the client declared.
    fn file_size(&self, hash: &ForgeHash) -> Option<u64>;

    /// Promote the listed hashes from staging into the live store.
    /// For FS backends this is a rename — atomic on the same volume.
    /// For S3, it's a CopyObject + DeleteObject pair.
    ///
    /// `live` is handed in rather than owned by the staging impl so
    /// a mixed deployment (FS staging + S3 live) is representable
    /// without a back-pointer mess.
    fn promote_into(
        &self,
        live: Arc<dyn ObjectBackend>,
        hashes: &[ForgeHash],
    ) -> io::Result<PromoteStats>;
}

/// Top-level repo-storage dispatcher. Hands out an object backend +
/// a staging handle for a given `(repo, session_id)`. Implementors
/// also carry the repo-lifecycle helpers (`rename_repo`,
/// `delete_repo`, `purge_session_staging`) so the gRPC CRUD
/// handlers don't need to know which backend is in play.
pub trait RepoStorageBackend: Send + Sync {
    /// The live (post-promote) object surface for `repo`.
    fn repo_object_backend(&self, repo: &str) -> Arc<dyn ObjectBackend>;

    /// Open a per-session staging handle. `session_id` must be
    /// pre-validated (`validate_session_id` in `services::grpc`) so
    /// implementors can treat it as trusted.
    fn session_staging(&self, repo: &str, session_id: &str) -> Box<dyn StagingBackend>;

    /// Clear a session's staging tree. Called by both the session
    /// sweeper (stale sessions) and the commit path (successful
    /// promote). Must tolerate a missing session.
    fn purge_session_staging(&self, repo: &str, session_id: &str) -> io::Result<()>;

    /// Rename the repo's on-disk (or in-bucket) layout. Filesystem
    /// backends use `std::fs::rename`; S3 backends copy + delete the
    /// whole prefix.
    fn rename_repo(&self, old_name: &str, new_name: &str) -> io::Result<()>;

    /// Delete the repo's layout entirely. Hard delete — cascades
    /// sit on the DB side.
    fn delete_repo(&self, name: &str) -> io::Result<()>;

    /// Local on-disk root of the repo's live store, if any. S3
    /// backends return `None`. Exists only because `ObjectStore` in
    /// forge-core is currently constructed from a local path; the
    /// Phase 3b.3 refactor teaches `ObjectStore` to accept an
    /// `Arc<dyn ObjectBackend>` directly, after which this escape
    /// hatch can be removed.
    fn repo_local_path(&self, _repo: &str) -> Option<PathBuf> {
        None
    }
}

// ── FsStorage impls ─────────────────────────────────────────────────────────

impl StagingBackend for StagingStore {
    fn ensure_shard_dirs(&self) -> io::Result<()> {
        StagingStore::ensure_shard_dirs(self)
    }

    fn put(&self, hash: &ForgeHash, data: &[u8]) -> io::Result<()> {
        StagingStore::put(self, hash, data)
    }

    fn append(&self, hash: &ForgeHash, data: &[u8]) -> io::Result<()> {
        StagingStore::append(self, hash, data)
    }

    fn file_size(&self, hash: &ForgeHash) -> Option<u64> {
        StagingStore::file_size(self, hash)
    }

    fn promote_into(
        &self,
        live: Arc<dyn ObjectBackend>,
        hashes: &[ForgeHash],
    ) -> io::Result<PromoteStats> {
        // Cross-backend path: read each staged file, put via the
        // live trait, then drop the staged copy. Works whether the
        // live side is the FS ChunkStore, an S3 object store, or
        // anything else that implements the trait.
        //
        // Trade-off vs the inherent `StagingStore::promote_into`:
        // that one uses `std::fs::rename` (metadata-only, O(1)) when
        // both sides live on the same FS. This trait impl copies the
        // bytes via put_raw. For the existing FS-FS hot path the
        // gRPC handler still calls the concrete `StagingStore::promote_into`
        // directly — the trait dispatch is reserved for
        // backend-neutral code paths (admin tooling, cross-backend
        // deployments). Phase 3b.3 will extend the live trait with
        // an abstract "atomic relocate" contract so this branch
        // can match the rename fast-path even through the trait.
        let mut stats = PromoteStats::default();
        for hash in hashes {
            let hex = hash.to_hex();
            let src = self.root().join(&hex[..2]).join(&hex[2..]);
            let raw = match std::fs::read(&src) {
                Ok(b) => b,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    stats.missing += 1;
                    continue;
                }
                Err(e) => return Err(e),
            };
            // If the live store already has it we just drop the
            // staged copy — content-addressed dedup.
            if live.has(hash) {
                let _ = std::fs::remove_file(&src);
                stats.deduped += 1;
                continue;
            }
            live.put_raw(hash, &raw).map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
            let _ = std::fs::remove_file(&src);
            stats.promoted += 1;
        }
        Ok(stats)
    }
}

impl RepoStorageBackend for FsStorage {
    fn repo_object_backend(&self, repo: &str) -> Arc<dyn ObjectBackend> {
        Arc::new(self.repo_store(repo))
    }

    fn session_staging(&self, repo: &str, session_id: &str) -> Box<dyn StagingBackend> {
        Box::new(self.session_staging_store(repo, session_id))
    }

    fn purge_session_staging(&self, repo: &str, session_id: &str) -> io::Result<()> {
        FsStorage::purge_session_staging(self, repo, session_id)
    }

    fn rename_repo(&self, old_name: &str, new_name: &str) -> io::Result<()> {
        FsStorage::rename_repo(self, old_name, new_name)
    }

    fn delete_repo(&self, name: &str) -> io::Result<()> {
        FsStorage::delete_repo(self, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::compress;

    #[test]
    fn fsstorage_satisfies_repo_storage_backend_trait() {
        // Smoke test: construct through the trait, push an object
        // through staging + promote, read back via live. Proves the
        // delegation compiles and runs end-to-end.
        let dir = tempfile::tempdir().unwrap();
        let fs: Arc<dyn RepoStorageBackend> =
            Arc::new(FsStorage::new(dir.path().to_path_buf(), Default::default()));

        let live = fs.repo_object_backend("alice/game");
        let staging = fs.session_staging("alice/game", "sid-1");
        staging.ensure_shard_dirs().unwrap();

        let payload = b"via-trait";
        let hash = ForgeHash::from_bytes(payload);
        let compressed = compress::compress(payload).unwrap();
        staging.put(&hash, &compressed).unwrap();
        assert_eq!(staging.file_size(&hash), Some(compressed.len() as u64));

        let stats = staging.promote_into(Arc::clone(&live), &[hash]).unwrap();
        assert_eq!(stats.promoted, 1);
        assert!(live.has(&hash));
    }

    #[test]
    fn purge_and_delete_flow_through_trait() {
        let dir = tempfile::tempdir().unwrap();
        let fs: Arc<dyn RepoStorageBackend> =
            Arc::new(FsStorage::new(dir.path().to_path_buf(), Default::default()));
        // Create some staging + live state …
        let live = fs.repo_object_backend("alice/game");
        live.put(
            &ForgeHash::from_bytes(b"live-obj"),
            b"live-obj",
        )
        .unwrap();
        let staging = fs.session_staging("alice/game", "sid-x");
        staging.ensure_shard_dirs().unwrap();

        // Purge the session; live data must survive.
        fs.purge_session_staging("alice/game", "sid-x").unwrap();
        assert!(live.has(&ForgeHash::from_bytes(b"live-obj")));

        // Delete the repo; the live object vanishes with its
        // directory.
        fs.delete_repo("alice/game").unwrap();
        // Reopening gives us a fresh empty backend.
        let reborn = fs.repo_object_backend("alice/game");
        assert!(!reborn.has(&ForgeHash::from_bytes(b"live-obj")));
    }
}
