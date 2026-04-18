// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

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
//! ## Sync trait with pooled off-runtime dispatch
//!
//! The trait stays **sync** deliberately. Making it async would
//! ripple through ~75+ call sites (grpc handlers, services, sweepers,
//! benches, parity tests, CLI admin, inline tests) and force every
//! sync caller to adopt `Runtime::block_on`. Measured overhead of the
//! existing dispatch today is **one channel round-trip** — the
//! `block_pg` helper used to spawn a fresh OS thread per call
//! (`std::thread::scope`, ~50-100 µs on Windows) but now submits
//! closures to a long-lived worker pool (see `block_pg` in
//! `storage::db`), so the marginal cost per call is a sync-channel
//! send + wake + receive (low single-digit µs).
//!
//! Full async conversion (trait + impls + call sites + tests) is
//! parked as a future refactor under the header "when HA deployment
//! forces it" — specifically, when measurable thread-pool starvation
//! shows up under real Postgres load. Until then, the pooled
//! dispatcher drops the only perf regression block_pg had.

use anyhow::Result;

use crate::storage::db::{
    CommitSessionOutcome, LockInfo, PendingRepoOp, RefUpdateSpec, RepoRecord, UploadSessionRecord,
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
    /// Batched variant used by the push hot path. A single transaction
    /// covers up to N `INSERT OR IGNORE` rows, which amortises SQLite's
    /// write-mutex cost across the batch (~100 µs → ~1 µs per row).
    /// The default impl just loops over [`Self::record_session_object`]
    /// so backends that can't implement a bulk-insert cheaply still
    /// compile, but both in-tree impls override it with a real bulk
    /// txn for the hot-path win.
    fn record_session_objects(&self, sid: &str, rows: &[(Vec<u8>, i64)]) -> Result<()> {
        for (h, s) in rows {
            self.record_session_object(sid, h, *s)?;
        }
        Ok(())
    }
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

    // -- Pending repo ops (Phase 3b.5 S3 drain queue) --

    /// Enqueue a durable repo-lifecycle op. `op_type` must be
    /// `"rename"` or `"delete"`; `new_repo` is the destination name
    /// for rename and ignored for delete. Returns the row id.
    fn enqueue_repo_op(&self, op_type: &str, repo: &str, new_repo: Option<&str>) -> Result<i64>;

    /// Claim the oldest eligible op. "Eligible" means `not_before <=
    /// now`; claiming bumps `not_before` to `now + visibility_secs`
    /// so a crashed worker's op becomes reclaimable after that
    /// window. Returns `None` when the queue is empty.
    fn claim_next_repo_op(&self, visibility_secs: i64) -> Result<Option<PendingRepoOp>>;

    /// Mark a claimed op complete — deletes the row.
    fn complete_repo_op(&self, id: i64) -> Result<()>;

    /// Mark a claimed op failed. Records `error` in `last_error` and
    /// pushes `not_before` out by `retry_delay_secs` so the drain
    /// backs off before retrying.
    fn fail_repo_op(&self, id: i64, error: &str, retry_delay_secs: i64) -> Result<()>;

    /// Snapshot every queued op for admin / debug tooling.
    fn list_pending_repo_ops(&self) -> Result<Vec<PendingRepoOp>>;

    // -- Schema versioning / migrations --

    /// Highest applied schema revision. 0 when no `schema_version`
    /// table exists yet.
    fn current_schema_version(&self) -> Result<i64>;

    /// Apply every pending migration for this backend. Each revision
    /// lands in its own transaction alongside the `schema_version`
    /// insert. Returns the number of migrations applied.
    fn apply_pending_migrations(&self) -> Result<usize>;

    // -- Health / observability --

    /// Liveness probe. Runs the cheapest possible round-trip so a
    /// wedged pool surfaces as a `/readyz` 503 instead of silently
    /// serving traffic.
    fn ping(&self) -> Result<()>;

    /// On-scrape counters for `/metrics`. One method per backend so
    /// the SQL stays dialect-native (SQLite COUNT(*) plans
    /// differently from Postgres COUNT(*) and we want both fast).
    fn metrics_snapshot(&self) -> Result<crate::storage::db::MetricsSnapshot>;
}
