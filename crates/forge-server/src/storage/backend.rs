// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Metadata backend trait.
//!
//! Narrow surface covering the Phase-1 atomic-push paths: repos, refs,
//! locks, and upload sessions. Auth/issues/PRs/workflows/actions/agents
//! intentionally remain on the concrete [`MetadataDb`] for now — Phase
//! 2b.2 avoids a Big-Bang refactor.
//!
//! Implementors live under `storage::db` (SQLite) and, behind the
//! `postgres` Cargo feature, `storage::postgres` (Postgres). Both
//! satisfy the same integration test suite so a regression in one
//! backend can't silently slip past the other.
//!
//! The trait is deliberately sync. rusqlite is sync; the `postgres`
//! crate is sync; most call-sites are sync. Async-ifying the surface
//! would touch ~400 LOC across `services/*` with no measured benefit
//! until Postgres starves the tokio thread pool under real load.

use anyhow::Result;

use crate::storage::db::{
    CommitSessionOutcome, LockInfo, RefUpdateSpec, RepoRecord, UploadSessionRecord,
};

/// Storage-backend abstraction for the Phase-1 atomic-push surface.
///
/// Every method here participates in either the push hot path
/// ([`Self::commit_upload_session`] and the ref/lock helpers it reads)
/// or the session sweeper. Adding a method is a load-bearing decision
/// — it forces both backends to grow a matching implementation.
pub trait MetadataBackend: Send + Sync {
    // -- Repos --

    fn list_repos(&self) -> Result<Vec<RepoRecord>>;
    fn get_repo_visibility(&self, name: &str) -> Result<Option<String>>;
    fn is_repo_public(&self, name: &str) -> bool;
    fn set_repo_visibility(&self, name: &str, visibility: &str) -> Result<bool>;
    fn create_repo(&self, name: &str, description: &str) -> Result<bool>;
    fn update_repo(&self, name: &str, new_name: &str, description: &str) -> Result<bool>;
    fn delete_repo(&self, name: &str) -> Result<bool>;

    // -- Refs --

    fn get_ref(&self, repo: &str, name: &str) -> Result<Option<Vec<u8>>>;
    fn get_all_refs(&self, repo: &str) -> Result<Vec<(String, Vec<u8>)>>;
    fn update_ref(
        &self,
        repo: &str,
        name: &str,
        old_hash: &[u8],
        new_hash: &[u8],
        force: bool,
    ) -> Result<bool>;

    // -- Locks --

    fn acquire_lock(
        &self,
        repo: &str,
        path: &str,
        owner: &str,
        workspace_id: &str,
        reason: &str,
    ) -> Result<std::result::Result<(), LockInfo>>;
    fn release_lock(&self, repo: &str, path: &str, owner: &str, force: bool) -> Result<bool>;
    fn list_locks(
        &self,
        repo: &str,
        path_prefix: &str,
        owner_filter: &str,
    ) -> Result<Vec<LockInfo>>;

    // -- Upload sessions (Phase-1 atomic push) --

    fn create_upload_session(
        &self,
        sid: &str,
        repo: &str,
        user_id: Option<i64>,
        ttl_seconds: i64,
    ) -> Result<()>;
    fn record_session_object(&self, sid: &str, hash: &[u8], size: i64) -> Result<()>;
    fn get_upload_session(&self, sid: &str) -> Result<Option<UploadSessionRecord>>;
    fn list_session_object_hashes(&self, sid: &str) -> Result<Vec<Vec<u8>>>;
    /// Hash + declared size per object recorded against the session.
    /// Used by the resume path to report how many bytes the server
    /// thinks each object is meant to be.
    fn list_session_objects_with_sizes(&self, sid: &str) -> Result<Vec<(Vec<u8>, i64)>>;
    fn fail_upload_session(&self, sid: &str, reason: &str, result_json: &str) -> Result<()>;
    fn commit_upload_session(
        &self,
        sid: &str,
        updates: &[RefUpdateSpec<'_>],
    ) -> Result<CommitSessionOutcome>;
    fn list_stale_upload_sessions(&self, cutoff_ts: i64) -> Result<Vec<(String, String)>>;
    fn delete_upload_session(&self, sid: &str) -> Result<()>;

    // -- Schema versioning / migrations --

    /// Highest applied schema revision. 0 when no `schema_version`
    /// table exists yet.
    fn current_schema_version(&self) -> Result<i64>;

    /// Apply every pending migration for this backend. Each revision
    /// lands in its own transaction alongside the `schema_version`
    /// insert. Returns the number of migrations applied.
    fn apply_pending_migrations(&self) -> Result<usize>;
}
