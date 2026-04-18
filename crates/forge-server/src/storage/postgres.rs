// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Postgres metadata backend for the Phase-1 atomic-push surface.
//!
//! Opt-in behind the `postgres` Cargo feature. Satisfies
//! [`crate::storage::backend::MetadataBackend`] — the same integration
//! tests that exercise the SQLite backend run against this one so a
//! regression can't slip past silently.
//!
//! Design notes:
//! - Sync `postgres` crate + r2d2 pool. Tokio never enters the storage
//!   layer; if Postgres starves the runtime under load we'll wrap at
//!   the RPC boundary with `spawn_blocking`.
//! - `statement_timeout` + `idle_in_transaction_session_timeout` are
//!   set on every pooled connection so a runaway query or an
//!   abandoned-mid-txn client doesn't pin a backend forever.
//! - Schema mirrors the SQLite baseline closely — BIGINT timestamps
//!   (epoch seconds, matching what the rest of the codebase passes
//!   through), BYTEA for hashes, explicit FK cascade.
//! - Migration runner uses the same append-only numbered list as
//!   SQLite; Phase 2b.1 established the pattern.

use anyhow::{Context, Result};
use postgres::{Client, Config as PgConfig, NoTls};
use r2d2::Pool;
use r2d2_postgres::PostgresConnectionManager;
use std::str::FromStr;
use std::time::Duration;

use crate::storage::backend::MetadataBackend;
use crate::storage::db::{
    CommitSessionOutcome, LockInfo, PendingRepoOp, RefUpdateOutcome, RefUpdateSpec, RepoRecord,
    UploadSessionRecord,
};

/// Tuning knobs for the Postgres pool. Defaults match the SQLite
/// path so operators don't re-learn two dialects of the same idea.
#[derive(Debug, Clone)]
pub struct PgPoolConfig {
    /// libpq-compatible connection URL (`postgres://user:pass@host/db`).
    pub url: String,
    /// Max pooled connections. Postgres handles real concurrency so
    /// the number can be higher than SQLite — 16 is a floor, not a
    /// ceiling, and stays consistent with the SQLite default.
    pub max_size: u32,
    /// `statement_timeout` (ms) applied at connection init.
    pub statement_timeout_ms: u64,
    /// `idle_in_transaction_session_timeout` (ms). Caps how long an
    /// abandoned mid-transaction connection can hold a backend before
    /// Postgres ROLLBACKs it.
    pub idle_in_txn_timeout_ms: u64,
}

impl Default for PgPoolConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            max_size: 16,
            statement_timeout_ms: 30_000,
            idle_in_txn_timeout_ms: 60_000,
        }
    }
}

/// Pool type alias. The `NoTls` connector matches the local-dev /
/// CI path — production deployments swap this for a rustls-based
/// manager once the TLS story for Postgres is in scope (Phase 7).
pub type PgPool = Pool<PostgresConnectionManager<NoTls>>;

/// Postgres-backed metadata store. Constructed by
/// [`PgMetadataBackend::open`]; ensures the baseline schema + any
/// pending migrations are applied before returning.
pub struct PgMetadataBackend {
    pool: PgPool,
    cfg: PgPoolConfig,
}

impl PgMetadataBackend {
    /// Open a pool, run pending migrations, return the backend.
    ///
    /// The timeouts travel as libpq-style `options` strings so they
    /// get installed as session GUCs the moment a physical connection
    /// is established. No per-query preamble required.
    pub fn open(cfg: PgPoolConfig) -> Result<Self> {
        if cfg.url.is_empty() {
            anyhow::bail!("postgres backend selected but [database] url is empty");
        }

        let mut pg_config = PgConfig::from_str(&cfg.url)
            .with_context(|| format!("parse postgres url: {}", cfg.url))?;

        // Install server-side timeouts as session GUCs. Forward-slashes
        // the ms values directly; libpq's `options` string accepts
        // `-c key=value` pairs separated by spaces (literal ' ').
        let options = format!(
            "-c statement_timeout={} -c idle_in_transaction_session_timeout={}",
            cfg.statement_timeout_ms, cfg.idle_in_txn_timeout_ms,
        );
        pg_config.options(&options);

        let manager = PostgresConnectionManager::new(pg_config, NoTls);
        let pool = Pool::builder()
            .max_size(cfg.max_size)
            .connection_timeout(Duration::from_secs(30))
            .build(manager)
            .context("build postgres pool")?;

        let backend = Self {
            pool,
            cfg: cfg.clone(),
        };
        backend
            .apply_pending_migrations_impl()
            .context("apply pending postgres migrations")?;
        Ok(backend)
    }

    /// Borrow a pooled client. Short-lived — call inside one method.
    fn conn(&self) -> Result<r2d2::PooledConnection<PostgresConnectionManager<NoTls>>> {
        self.pool.get().context("postgres pool get")
    }

    fn apply_pending_migrations_impl(&self) -> Result<usize> {
        let current = self.current_schema_version_impl()?;
        let mut conn = self.conn()?;
        crate::storage::migrations::apply_pending_postgres(
            &mut *conn,
            current,
            crate::storage::migrations::POSTGRES_MIGRATIONS,
        )
    }

    fn current_schema_version_impl(&self) -> Result<i64> {
        let mut conn = self.conn()?;
        // The table may not exist on a fresh database — treat that as
        // version 0 so the runner's "apply everything" path kicks in.
        let exists: bool = conn
            .query_one(
                "SELECT EXISTS (
                    SELECT 1 FROM information_schema.tables
                    WHERE table_name = 'schema_version'
                )",
                &[],
            )
            .context("check schema_version table")?
            .get(0);
        if !exists {
            return Ok(0);
        }
        let v: Option<i64> = conn
            .query_one("SELECT MAX(version) FROM schema_version", &[])
            .context("read current schema_version")?
            .get(0);
        Ok(v.unwrap_or(0))
    }

    /// Connection string used at open time. Exposed for debugging /
    /// test harnesses that want to spawn additional admin sessions
    /// against the same database.
    #[allow(dead_code)]
    pub fn url(&self) -> &str {
        &self.cfg.url
    }
}

// -- Trait impl --
//
// Each method opens a pooled connection, runs the SQL, maps the row.
// Deliberately verbose to keep the mapping logic obvious — every
// SQLite query has a line-for-line equivalent here, which is the
// property the cross-backend integration tests rely on.

impl MetadataBackend for PgMetadataBackend {
    // -- Repos --

    fn list_repos(&self) -> Result<Vec<RepoRecord>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            "SELECT name, description, created_at, visibility, default_branch FROM repos",
            &[],
        )?;
        Ok(rows
            .into_iter()
            .map(|r| RepoRecord {
                name: r.get(0),
                description: r.get(1),
                created_at: r.get(2),
                visibility: r.get(3),
                default_branch: r.get(4),
            })
            .collect())
    }

    fn get_repo_visibility(&self, name: &str) -> Result<Option<String>> {
        let mut conn = self.conn()?;
        let row = conn.query_opt("SELECT visibility FROM repos WHERE name = $1", &[&name])?;
        Ok(row.map(|r| r.get(0)))
    }

    fn is_repo_public(&self, name: &str) -> bool {
        matches!(
            self.get_repo_visibility(name).ok().flatten().as_deref(),
            Some("public"),
        )
    }

    fn set_repo_visibility(&self, name: &str, visibility: &str) -> Result<bool> {
        if visibility != "private" && visibility != "public" {
            anyhow::bail!("visibility must be 'private' or 'public'");
        }
        let mut conn = self.conn()?;
        let n = conn.execute(
            "UPDATE repos SET visibility = $1 WHERE name = $2",
            &[&visibility, &name],
        )?;
        Ok(n > 0)
    }

    fn create_repo(&self, name: &str, description: &str) -> Result<bool> {
        let mut conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        let n = conn.execute(
            "INSERT INTO repos (name, description, created_at)
             VALUES ($1, $2, $3)
             ON CONFLICT (name) DO NOTHING",
            &[&name, &description, &now],
        )?;
        Ok(n > 0)
    }

    fn update_repo(&self, name: &str, new_name: &str, description: &str) -> Result<bool> {
        let mut conn = self.conn()?;

        let exists: i64 = conn
            .query_one("SELECT COUNT(*) FROM repos WHERE name = $1", &[&name])?
            .get(0);
        if exists == 0 {
            return Ok(false);
        }

        let effective_name = if new_name.is_empty() { name } else { new_name };

        if !new_name.is_empty() && new_name != name {
            let taken: i64 = conn
                .query_one("SELECT COUNT(*) FROM repos WHERE name = $1", &[&new_name])?
                .get(0);
            if taken > 0 {
                anyhow::bail!("repo '{}' already exists", new_name);
            }
        }

        let mut tx = conn.transaction()?;
        tx.execute(
            "UPDATE repos SET name = $1, description = $2 WHERE name = $3",
            &[&effective_name, &description, &name],
        )?;
        if !new_name.is_empty() && new_name != name {
            tx.execute(
                "UPDATE refs SET repo = $1 WHERE repo = $2",
                &[&new_name, &name],
            )?;
            tx.execute(
                "UPDATE locks SET repo = $1 WHERE repo = $2",
                &[&new_name, &name],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    fn delete_repo(&self, name: &str) -> Result<bool> {
        let mut conn = self.conn()?;
        let mut tx = conn.transaction()?;
        let affected = tx.execute("DELETE FROM repos WHERE name = $1", &[&name])?;
        // ON DELETE CASCADE cleans refs + locks. Keep the explicit
        // DELETEs as a belt-and-braces no-op in case an operator drops
        // the FK in a future migration.
        tx.execute("DELETE FROM refs WHERE repo = $1", &[&name])?;
        tx.execute("DELETE FROM locks WHERE repo = $1", &[&name])?;
        tx.commit()?;
        Ok(affected > 0)
    }

    // -- Refs --

    fn get_ref(&self, repo: &str, name: &str) -> Result<Option<Vec<u8>>> {
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            "SELECT hash FROM refs WHERE repo = $1 AND name = $2",
            &[&repo, &name],
        )?;
        Ok(row.map(|r| r.get(0)))
    }

    fn get_all_refs(&self, repo: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let mut conn = self.conn()?;
        let rows = conn.query("SELECT name, hash FROM refs WHERE repo = $1", &[&repo])?;
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    fn update_ref(
        &self,
        repo: &str,
        name: &str,
        old_hash: &[u8],
        new_hash: &[u8],
        force: bool,
    ) -> Result<bool> {
        let mut conn = self.conn()?;
        if force {
            let n = conn.execute(
                "INSERT INTO refs (repo, name, hash) VALUES ($1, $2, $3)
                 ON CONFLICT (repo, name) DO UPDATE SET hash = EXCLUDED.hash",
                &[&repo, &name, &new_hash],
            )?;
            return Ok(n > 0);
        }

        let is_create = old_hash.iter().all(|&b| b == 0);
        let affected = if is_create {
            conn.execute(
                "INSERT INTO refs (repo, name, hash) VALUES ($1, $2, $3)
                 ON CONFLICT (repo, name) DO NOTHING",
                &[&repo, &name, &new_hash],
            )?
        } else {
            conn.execute(
                "UPDATE refs SET hash = $1 WHERE repo = $2 AND name = $3 AND hash = $4",
                &[&new_hash, &repo, &name, &old_hash],
            )?
        };
        Ok(affected > 0)
    }

    // -- Locks --

    fn acquire_lock(
        &self,
        repo: &str,
        path: &str,
        owner: &str,
        workspace_id: &str,
        reason: &str,
    ) -> Result<std::result::Result<(), LockInfo>> {
        let mut conn = self.conn()?;

        if let Some(row) = conn.query_opt(
            "SELECT owner, workspace_id, created_at, reason
             FROM locks WHERE repo = $1 AND path = $2",
            &[&repo, &path],
        )? {
            let existing = LockInfo {
                path: path.to_string(),
                owner: row.get(0),
                workspace_id: row.get(1),
                created_at: row.get(2),
                reason: row.get::<_, Option<String>>(3).unwrap_or_default(),
            };
            if existing.owner == owner {
                return Ok(Ok(()));
            }
            return Ok(Err(existing));
        }

        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO locks (repo, path, owner, workspace_id, created_at, reason)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[&repo, &path, &owner, &workspace_id, &now, &reason],
        )?;
        Ok(Ok(()))
    }

    fn release_lock(&self, repo: &str, path: &str, owner: &str, force: bool) -> Result<bool> {
        let mut conn = self.conn()?;
        let affected = if force {
            conn.execute(
                "DELETE FROM locks WHERE repo = $1 AND path = $2",
                &[&repo, &path],
            )?
        } else {
            conn.execute(
                "DELETE FROM locks WHERE repo = $1 AND path = $2 AND owner = $3",
                &[&repo, &path, &owner],
            )?
        };
        Ok(affected > 0)
    }

    fn list_locks(
        &self,
        repo: &str,
        path_prefix: &str,
        owner_filter: &str,
    ) -> Result<Vec<LockInfo>> {
        let mut conn = self.conn()?;

        let prefix_pattern = if path_prefix.is_empty() {
            "%".to_string()
        } else {
            format!("{path_prefix}%")
        };
        let owner_pattern = if owner_filter.is_empty() {
            "%".to_string()
        } else {
            owner_filter.to_string()
        };

        let rows = conn.query(
            "SELECT path, owner, workspace_id, created_at, reason
             FROM locks
             WHERE repo = $1 AND path LIKE $2 AND owner LIKE $3
             LIMIT 10000",
            &[&repo, &prefix_pattern, &owner_pattern],
        )?;

        Ok(rows
            .into_iter()
            .map(|r| LockInfo {
                path: r.get(0),
                owner: r.get(1),
                workspace_id: r.get(2),
                created_at: r.get(3),
                reason: r.get::<_, Option<String>>(4).unwrap_or_default(),
            })
            .collect())
    }

    // -- Upload sessions --

    fn create_upload_session(
        &self,
        sid: &str,
        repo: &str,
        user_id: Option<i64>,
        ttl_seconds: i64,
    ) -> Result<()> {
        let mut conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        let expires = now + ttl_seconds;
        conn.execute(
            "INSERT INTO upload_sessions
                (id, repo, user_id, state, created_at, expires_at)
             VALUES ($1, $2, $3, 'uploading', $4, $5)
             ON CONFLICT (id) DO NOTHING",
            &[&sid, &repo, &user_id, &now, &expires],
        )?;
        Ok(())
    }

    fn record_session_object(&self, sid: &str, hash: &[u8], size: i64) -> Result<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "INSERT INTO session_objects (session_id, hash, size)
             VALUES ($1, $2, $3)
             ON CONFLICT (session_id, hash) DO NOTHING",
            &[&sid, &hash, &size],
        )?;
        Ok(())
    }

    fn get_upload_session(&self, sid: &str) -> Result<Option<UploadSessionRecord>> {
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            "SELECT id, repo, user_id, state, created_at, expires_at,
                    committed_at, result_json, failure
             FROM upload_sessions WHERE id = $1",
            &[&sid],
        )?;
        Ok(row.map(|r| UploadSessionRecord {
            id: r.get(0),
            repo: r.get(1),
            user_id: r.get(2),
            state: r.get(3),
            created_at: r.get(4),
            expires_at: r.get(5),
            committed_at: r.get(6),
            result_json: r.get(7),
            failure: r.get(8),
        }))
    }

    fn list_session_object_hashes(&self, sid: &str) -> Result<Vec<Vec<u8>>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            "SELECT hash FROM session_objects WHERE session_id = $1",
            &[&sid],
        )?;
        Ok(rows.into_iter().map(|r| r.get::<_, Vec<u8>>(0)).collect())
    }

    fn list_session_objects_with_sizes(&self, sid: &str) -> Result<Vec<(Vec<u8>, i64)>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            "SELECT hash, size FROM session_objects WHERE session_id = $1",
            &[&sid],
        )?;
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    fn fail_upload_session(&self, sid: &str, reason: &str, result_json: &str) -> Result<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "UPDATE upload_sessions
             SET state = 'failed', failure = $2, result_json = $3
             WHERE id = $1 AND state = 'uploading'",
            &[&sid, &reason, &result_json],
        )?;
        Ok(())
    }

    fn commit_upload_session(
        &self,
        sid: &str,
        updates: &[RefUpdateSpec<'_>],
    ) -> Result<CommitSessionOutcome> {
        let mut conn = self.conn()?;
        let mut tx = conn.transaction()?;

        let session_row = tx.query_opt(
            "SELECT state, result_json FROM upload_sessions WHERE id = $1",
            &[&sid],
        )?;

        let (state, cached_result) = match session_row {
            Some(r) => (r.get::<_, String>(0), r.get::<_, Option<String>>(1)),
            None => return Ok(CommitSessionOutcome::Unknown),
        };

        match state.as_str() {
            "committed" => {
                return Ok(CommitSessionOutcome::AlreadyCommitted {
                    result_json: cached_result.unwrap_or_default(),
                });
            }
            "failed" | "abandoned" => {
                return Ok(CommitSessionOutcome::TerminallyFailed {
                    reason: state,
                    result_json: cached_result.unwrap_or_default(),
                });
            }
            "uploading" => {}
            other => anyhow::bail!("unexpected upload session state: {other}"),
        }

        let repo: String = tx
            .query_one("SELECT repo FROM upload_sessions WHERE id = $1", &[&sid])?
            .get(0);

        let mut ref_results = Vec::with_capacity(updates.len());
        for u in updates {
            let success = apply_ref_update_pg_tx(&mut tx, &repo, u)?;
            let error = if success {
                String::new()
            } else if u.force {
                "force update failed".to_string()
            } else if u.old_hash.iter().all(|&b| b == 0) {
                "ref already exists".to_string()
            } else {
                "ref has been updated by another client".to_string()
            };
            ref_results.push(RefUpdateOutcome {
                ref_name: u.ref_name.to_string(),
                success,
                error,
            });
        }

        let all_success = ref_results.iter().all(|r| r.success);
        let now = chrono::Utc::now().timestamp();
        let result_json = serde_json::to_string(&ref_results).unwrap_or_else(|_| "[]".into());

        if all_success {
            tx.execute(
                "UPDATE upload_sessions
                 SET state = 'committed', committed_at = $2, result_json = $3
                 WHERE id = $1",
                &[&sid, &now, &result_json],
            )?;
        }
        // Otherwise leave session state = 'uploading' so the client can
        // rebase + retry without losing staged objects. The sweeper
        // reclaims it if the retry never arrives.

        tx.commit()?;

        Ok(CommitSessionOutcome::Committed {
            ref_results,
            all_success,
        })
    }

    fn list_stale_upload_sessions(&self, cutoff_ts: i64) -> Result<Vec<(String, String)>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            "SELECT id, repo FROM upload_sessions
             WHERE (state = 'uploading' AND expires_at <= $1)
                OR (state IN ('failed','abandoned','committed') AND
                    COALESCE(committed_at, created_at) <= $1)",
            &[&cutoff_ts],
        )?;
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    fn delete_upload_session(&self, sid: &str) -> Result<()> {
        let mut conn = self.conn()?;
        conn.execute("DELETE FROM upload_sessions WHERE id = $1", &[&sid])?;
        Ok(())
    }

    // -- Pending repo ops (Phase 3b.5) --

    fn enqueue_repo_op(&self, op_type: &str, repo: &str, new_repo: Option<&str>) -> Result<i64> {
        if op_type != "rename" && op_type != "delete" {
            anyhow::bail!("op_type must be 'rename' or 'delete', got '{op_type}'");
        }
        let mut conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        let row = conn.query_one(
            "INSERT INTO pending_repo_ops
                (op_type, repo, new_repo, created_at, not_before, attempts)
             VALUES ($1, $2, $3, $4, 0, 0)
             RETURNING id",
            &[&op_type, &repo, &new_repo, &now],
        )?;
        Ok(row.get(0))
    }

    fn claim_next_repo_op(&self, visibility_secs: i64) -> Result<Option<PendingRepoOp>> {
        let mut conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        let next_visible = now + visibility_secs;
        // SKIP LOCKED lets multiple drain workers dequeue concurrently
        // without starving each other on the same row.
        let row = conn.query_opt(
            "UPDATE pending_repo_ops
             SET not_before = $1, attempts = attempts + 1
             WHERE id = (
                 SELECT id FROM pending_repo_ops
                 WHERE not_before <= $2
                 ORDER BY created_at
                 FOR UPDATE SKIP LOCKED
                 LIMIT 1
             )
             RETURNING id, op_type, repo, new_repo, attempts",
            &[&next_visible, &now],
        )?;
        Ok(row.map(|r| PendingRepoOp {
            id: r.get(0),
            op_type: r.get(1),
            repo: r.get(2),
            new_repo: r.get(3),
            attempts: r.get(4),
        }))
    }

    fn complete_repo_op(&self, id: i64) -> Result<()> {
        let mut conn = self.conn()?;
        conn.execute("DELETE FROM pending_repo_ops WHERE id = $1", &[&id])?;
        Ok(())
    }

    fn fail_repo_op(&self, id: i64, error: &str, retry_delay_secs: i64) -> Result<()> {
        let mut conn = self.conn()?;
        let retry_at = chrono::Utc::now().timestamp() + retry_delay_secs;
        conn.execute(
            "UPDATE pending_repo_ops
             SET last_error = $2, not_before = $3
             WHERE id = $1",
            &[&id, &error, &retry_at],
        )?;
        Ok(())
    }

    fn list_pending_repo_ops(&self) -> Result<Vec<PendingRepoOp>> {
        let mut conn = self.conn()?;
        let rows = conn.query(
            "SELECT id, op_type, repo, new_repo, attempts
             FROM pending_repo_ops
             ORDER BY created_at",
            &[],
        )?;
        Ok(rows
            .into_iter()
            .map(|r| PendingRepoOp {
                id: r.get(0),
                op_type: r.get(1),
                repo: r.get(2),
                new_repo: r.get(3),
                attempts: r.get(4),
            })
            .collect())
    }

    // -- Schema versioning --

    fn current_schema_version(&self) -> Result<i64> {
        self.current_schema_version_impl()
    }

    fn apply_pending_migrations(&self) -> Result<usize> {
        self.apply_pending_migrations_impl()
    }
}

/// Apply one ref update inside an active transaction. Mirrors the
/// SQLite helper in `db.rs` — same semantics, same decision tree,
/// Postgres dialect. Scoped to this module because callers route
/// through [`PgMetadataBackend::commit_upload_session`].
fn apply_ref_update_pg_tx(
    tx: &mut postgres::Transaction<'_>,
    repo: &str,
    u: &RefUpdateSpec<'_>,
) -> Result<bool> {
    if u.force {
        let affected = tx.execute(
            "INSERT INTO refs (repo, name, hash) VALUES ($1, $2, $3)
             ON CONFLICT (repo, name) DO UPDATE SET hash = EXCLUDED.hash",
            &[&repo, &u.ref_name, &u.new_hash],
        )?;
        return Ok(affected > 0);
    }

    let is_create = u.old_hash.iter().all(|&b| b == 0);
    let affected = if is_create {
        tx.execute(
            "INSERT INTO refs (repo, name, hash) VALUES ($1, $2, $3)
             ON CONFLICT (repo, name) DO NOTHING",
            &[&repo, &u.ref_name, &u.new_hash],
        )?
    } else {
        tx.execute(
            "UPDATE refs SET hash = $1 WHERE repo = $2 AND name = $3 AND hash = $4",
            &[&u.new_hash, &repo, &u.ref_name, &u.old_hash],
        )?
    };
    Ok(affected > 0)
}
