// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{Context, Result};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;

/// Pooled connection handle returned by [`MetadataDb::conn`].
///
/// Derefs to [`rusqlite::Connection`] so existing call sites keep
/// working unchanged. Carries its own busy-timeout and pragmas —
/// installed by the pool's `on_acquire` hook so every hand-out is
/// correctly configured, even after the pool spins up a new physical
/// connection to satisfy demand.
pub(crate) type PooledConn = r2d2::PooledConnection<SqliteConnectionManager>;

/// Connection-pool tuning baked into `MetadataDb::open`. These are
/// sane defaults for a single-host server; the operator can override
/// them via the `[database]` config block (see `ServerConfig`).
#[derive(Debug, Clone, Copy)]
pub struct DbPoolConfig {
    /// Upper bound on pooled connections. SQLite serialises writes
    /// regardless, but additional readers multiply WAL read throughput
    /// and keep handler tasks from blocking on a single connection.
    pub max_size: u32,
    /// Per-connection `PRAGMA busy_timeout` in milliseconds. Controls
    /// how long SQLite waits for the write lock before returning
    /// `SQLITE_BUSY`. Pool-level wait is separate (r2d2's own timeout).
    pub busy_timeout_ms: u64,
}

impl Default for DbPoolConfig {
    fn default() -> Self {
        Self {
            max_size: 16,
            busy_timeout_ms: 5_000,
        }
    }
}

/// SQLite-backed metadata store for repos, refs, locks, upload
/// sessions, and auth. Backed by an r2d2 pool so metadata ops don't
/// serialise on a single Mutex — with WAL + `BEGIN IMMEDIATE` +
/// `synchronous = NORMAL` this lifts the pre-Phase-2 bottleneck that
/// capped throughput at ~10 concurrent clients.
pub struct MetadataDb {
    pub(crate) pool: Pool<SqliteConnectionManager>,
}

impl MetadataDb {
    /// Fetch a pooled connection. Blocks up to the r2d2 wait timeout
    /// (currently the default 30 s) if all connections are in use —
    /// should be rare on a correctly-sized pool, and a timeout here
    /// indicates either runaway concurrency or a stuck long-held txn.
    pub(crate) fn conn(&self) -> Result<PooledConn> {
        self.pool
            .get()
            .map_err(|e| anyhow::anyhow!("metadata pool get: {e}"))
    }

    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_config(path, DbPoolConfig::default())
    }

    /// Open a pooled SQLite metadata store. The `on_acquire` callback
    /// runs against every connection the pool produces, so pragmas
    /// stick even when r2d2 grows the pool to handle a burst. Schema
    /// creation is a single execute_batch on a connection borrowed
    /// from the freshly-built pool.
    pub fn open_with_config(path: &Path, cfg: DbPoolConfig) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let busy_timeout_ms = cfg.busy_timeout_ms;
        let manager = SqliteConnectionManager::file(path)
            .with_flags(
                OpenFlags::SQLITE_OPEN_READ_WRITE
                    | OpenFlags::SQLITE_OPEN_CREATE
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .with_init(move |c: &mut Connection| {
                // Per-connection pragmas. Applied before any query runs.
                c.busy_timeout(Duration::from_millis(busy_timeout_ms))?;
                // WAL + NORMAL = one writer + many readers without the
                // fsync-per-txn cost of the FULL mode. Survivable
                // across crashes because the WAL itself is fsync'd on
                // checkpoint boundaries.
                c.pragma_update(None, "journal_mode", "WAL")?;
                c.pragma_update(None, "synchronous", "NORMAL")?;
                // Automatic WAL checkpoint every ~1000 pages so long-
                // running servers don't accumulate a multi-gig WAL.
                c.pragma_update(None, "wal_autocheckpoint", 1000)?;
                // FK enforcement is off by default per-connection.
                // Auth tables depend on ON DELETE CASCADE; must be on
                // for every hand-out from the pool, not just the
                // connection used at open time.
                c.pragma_update(None, "foreign_keys", "ON")?;
                Ok(())
            });

        let pool = Pool::builder()
            .max_size(cfg.max_size)
            // Test the connection on checkout so a stale FD (e.g.
            // after a DB file replace) doesn't poison a handler.
            .test_on_check_out(true)
            .build(manager)
            .with_context(|| {
                format!("Failed to build metadata pool for {}", path.display())
            })?;

        // Schema setup runs on a freshly pooled connection. The pragmas
        // above have already been applied via `with_init`.
        let conn = pool
            .get()
            .with_context(|| "initial pool.get() failed during open()")?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS repos (
                name TEXT PRIMARY KEY,
                description TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL,
                visibility TEXT NOT NULL DEFAULT 'private'
                    CHECK(visibility IN ('private','public'))
            );
            CREATE TABLE IF NOT EXISTS refs (
                repo TEXT NOT NULL,
                name TEXT NOT NULL,
                hash BLOB NOT NULL,
                PRIMARY KEY (repo, name)
            );
            CREATE TABLE IF NOT EXISTS locks (
                repo TEXT NOT NULL,
                path TEXT NOT NULL,
                owner TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                reason TEXT,
                PRIMARY KEY (repo, path)
            );
            CREATE TABLE IF NOT EXISTS users (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                username        TEXT    NOT NULL UNIQUE,
                email           TEXT    NOT NULL UNIQUE,
                display_name    TEXT    NOT NULL,
                password_hash   TEXT,
                is_server_admin INTEGER NOT NULL DEFAULT 0,
                created_at      INTEGER NOT NULL,
                last_login_at   INTEGER
            );
            CREATE TABLE IF NOT EXISTS sessions (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                token_hash   TEXT    NOT NULL UNIQUE,
                token_prefix TEXT    NOT NULL,
                user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                created_at   INTEGER NOT NULL,
                last_used_at INTEGER NOT NULL,
                expires_at   INTEGER NOT NULL,
                user_agent   TEXT,
                ip           TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_prefix ON sessions(token_prefix);
            CREATE TABLE IF NOT EXISTS personal_access_tokens (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                name         TEXT    NOT NULL,
                token_hash   TEXT    NOT NULL UNIQUE,
                token_prefix TEXT    NOT NULL,
                user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                scopes       TEXT    NOT NULL,
                created_at   INTEGER NOT NULL,
                last_used_at INTEGER,
                expires_at   INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_pats_user ON personal_access_tokens(user_id);
            CREATE INDEX IF NOT EXISTS idx_pats_prefix ON personal_access_tokens(token_prefix);
            CREATE TABLE IF NOT EXISTS repo_acls (
                repo       TEXT    NOT NULL,
                user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                role       TEXT    NOT NULL CHECK(role IN ('read','write','admin')),
                granted_at INTEGER NOT NULL,
                granted_by INTEGER REFERENCES users(id),
                PRIMARY KEY (repo, user_id)
            );
            CREATE INDEX IF NOT EXISTS idx_repo_acls_user ON repo_acls(user_id);
            ",
        )?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS issues (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo TEXT NOT NULL,
                title TEXT NOT NULL,
                body TEXT NOT NULL DEFAULT '',
                author TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'open',
                labels TEXT NOT NULL DEFAULT '',
                assignee TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                comment_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS pull_requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo TEXT NOT NULL,
                title TEXT NOT NULL,
                body TEXT NOT NULL DEFAULT '',
                author TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'open',
                source_branch TEXT NOT NULL,
                target_branch TEXT NOT NULL DEFAULT 'main',
                labels TEXT NOT NULL DEFAULT '',
                assignee TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                comment_count INTEGER NOT NULL DEFAULT 0
            );
            ",
        )?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS comments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo TEXT NOT NULL,
                issue_id INTEGER NOT NULL,
                kind TEXT NOT NULL DEFAULT 'issue' CHECK(kind IN ('issue','pull_request')),
                author TEXT NOT NULL,
                body TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_comments_issue ON comments(repo, issue_id, kind);
            ",
        )?;

        // Migrate: add assignee column if missing
        let _ = conn.execute("ALTER TABLE issues ADD COLUMN assignee TEXT NOT NULL DEFAULT ''", []);
        let _ = conn.execute("ALTER TABLE pull_requests ADD COLUMN assignee TEXT NOT NULL DEFAULT ''", []);
        // Migrate: add default_branch column if missing
        let _ = conn.execute("ALTER TABLE repos ADD COLUMN default_branch TEXT NOT NULL DEFAULT ''", []);

        // Release the initial connection back to the pool so subsequent
        // setup calls don't deadlock when the pool is small.
        drop(conn);

        let db = Self { pool };
        db.ensure_schema_version_table()?;
        db.create_actions_tables()?;
        db.create_secrets_tables()?;
        db.create_agent_tables()?;
        db.create_upload_session_tables()?;
        // Record the baseline schema so Phase 2b migrations can version
        // their add-only DDL against a concrete starting point.
        db.record_baseline_schema_version()?;
        // Apply any numbered migrations beyond the baseline. Errors
        // propagate — a half-applied schema is worse than a server
        // that refuses to start.
        db.apply_pending_migrations()?;
        Ok(db)
    }

    /// Run every SQLite migration whose version is greater than the
    /// currently-recorded schema_version. Each migration lands in its
    /// own BEGIN IMMEDIATE transaction alongside the `schema_version`
    /// insert, so a crash in the middle of a migration leaves the DB
    /// on the previous revision (never half-applied).
    fn apply_pending_migrations(&self) -> Result<()> {
        let current = self.current_schema_version()?;
        let mut conn = self.conn()?;
        let applied = crate::storage::migrations::apply_pending(
            &mut conn,
            current,
            crate::storage::migrations::SQLITE_MIGRATIONS,
        )?;
        if applied == 0 {
            tracing::debug!(
                current_version = current,
                "no pending migrations"
            );
        }
        Ok(())
    }

    // -- Schema versioning --
    //
    // Every server start runs `ensure_schema_version_table()` and any
    // pending migrations. The current file is treated as revision 1
    // (the "baseline" captured at Phase 2a); add-only DDL for Phase 2b
    // onwards will land as numbered migrations that this module applies
    // idempotently at boot.

    fn ensure_schema_version_table(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version     INTEGER PRIMARY KEY,
                name        TEXT    NOT NULL,
                applied_at  INTEGER NOT NULL
            );",
        )?;
        Ok(())
    }

    fn record_baseline_schema_version(&self) -> Result<()> {
        let conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR IGNORE INTO schema_version (version, name, applied_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![1i64, "baseline", now],
        )?;
        Ok(())
    }

    /// Return the highest applied schema version. Used by the Phase 2b
    /// migration runner to skip already-applied steps.
    #[allow(dead_code)]
    pub(crate) fn current_schema_version(&self) -> Result<i64> {
        let conn = self.conn()?;
        let v: i64 = conn
            .prepare("SELECT COALESCE(MAX(version), 0) FROM schema_version")?
            .query_row([], |r| r.get(0))?;
        Ok(v)
    }

    // -- Upload sessions (Phase 1 atomic push) --
    //
    // A push is a two-phase operation: PushObjects streams bytes into a
    // per-session staging area, and CommitPush promotes those bytes into the
    // live tree plus applies ref CAS updates inside a single transaction.
    // These two tables track the session so CommitPush retries stay
    // idempotent and a sweeper can reclaim staging from clients that crashed
    // mid-push.

    pub fn create_upload_session_tables(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS upload_sessions (
                id           TEXT PRIMARY KEY,
                repo         TEXT NOT NULL,
                user_id      INTEGER,
                state        TEXT NOT NULL
                    CHECK(state IN ('uploading','committed','failed','abandoned'))
                    DEFAULT 'uploading',
                created_at   INTEGER NOT NULL,
                expires_at   INTEGER NOT NULL,
                committed_at INTEGER,
                -- JSON-encoded CommitPush outcome captured at commit time so
                -- an idempotent retry returns the same response.
                result_json  TEXT,
                -- Free-form error label when state='failed'. Not shown to
                -- clients directly; useful for operator debugging.
                failure      TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_upload_sessions_state
                ON upload_sessions(state, expires_at);
            CREATE TABLE IF NOT EXISTS session_objects (
                session_id TEXT NOT NULL
                    REFERENCES upload_sessions(id) ON DELETE CASCADE,
                hash       BLOB NOT NULL,
                size       INTEGER NOT NULL,
                PRIMARY KEY (session_id, hash)
            );
            CREATE INDEX IF NOT EXISTS idx_session_objects_session
                ON session_objects(session_id);
            ",
        )?;
        Ok(())
    }

    /// Create an upload session if it doesn't already exist. No-op on a
    /// duplicate id so retried first-chunks (same session) are safe.
    pub fn create_upload_session(
        &self,
        sid: &str,
        repo: &str,
        user_id: Option<i64>,
        ttl_seconds: i64,
    ) -> Result<()> {
        let conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        let expires = now + ttl_seconds;
        conn.execute(
            "INSERT OR IGNORE INTO upload_sessions
                (id, repo, user_id, state, created_at, expires_at)
             VALUES (?1, ?2, ?3, 'uploading', ?4, ?5)",
            rusqlite::params![sid, repo, user_id, now, expires],
        )?;
        Ok(())
    }

    /// Record a successfully-staged object against a session. `OR IGNORE`
    /// because a client resending a chunk for dedup is fine.
    pub fn record_session_object(&self, sid: &str, hash: &[u8], size: i64) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO session_objects (session_id, hash, size)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![sid, hash, size],
        )?;
        Ok(())
    }

    pub fn get_upload_session(&self, sid: &str) -> Result<Option<UploadSessionRecord>> {
        let conn = self.conn()?;
        let row = conn
            .prepare(
                "SELECT id, repo, user_id, state, created_at, expires_at,
                        committed_at, result_json, failure
                 FROM upload_sessions WHERE id = ?1",
            )?
            .query_row([sid], |r| {
                Ok(UploadSessionRecord {
                    id: r.get(0)?,
                    repo: r.get(1)?,
                    user_id: r.get(2)?,
                    state: r.get(3)?,
                    created_at: r.get(4)?,
                    expires_at: r.get(5)?,
                    committed_at: r.get(6)?,
                    result_json: r.get(7)?,
                    failure: r.get(8)?,
                })
            })
            .ok();
        Ok(row)
    }

    pub fn list_session_object_hashes(&self, sid: &str) -> Result<Vec<Vec<u8>>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT hash FROM session_objects WHERE session_id = ?1")?;
        let rows = stmt.query_map([sid], |r| r.get::<_, Vec<u8>>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Hashes + declared sizes for every object recorded against a
    /// session. Used by `QueryUploadSession` to tell the resuming
    /// client how many bytes per object it's already announced — the
    /// staging filesystem answers the "how many did I actually land"
    /// question separately.
    pub fn list_session_objects_with_sizes(&self, sid: &str) -> Result<Vec<(Vec<u8>, i64)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT hash, size FROM session_objects WHERE session_id = ?1",
        )?;
        let rows = stmt.query_map([sid], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Mark a session failed. Retries of CommitPush against a failed session
    /// will surface the same failure reason without re-running the work.
    pub fn fail_upload_session(&self, sid: &str, reason: &str, result_json: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE upload_sessions
             SET state = 'failed', failure = ?2, result_json = ?3
             WHERE id = ?1 AND state = 'uploading'",
            rusqlite::params![sid, reason, result_json],
        )?;
        Ok(())
    }

    /// Atomic commit: inside a single `BEGIN IMMEDIATE`, verify the session
    /// is still in `uploading` state, apply each ref update (with the same
    /// CAS / create / force semantics as `update_ref`), capture per-ref
    /// outcomes, and mark the session committed. On retry against an
    /// already-committed session, returns the cached result without doing
    /// any writes.
    ///
    /// The caller is responsible for having already promoted objects from
    /// staging → live before invoking this. Ref updates that reference
    /// missing objects will fail the reachability check higher in the stack.
    pub fn commit_upload_session(
        &self,
        sid: &str,
        updates: &[RefUpdateSpec<'_>],
    ) -> Result<CommitSessionOutcome> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let session: Option<(String, Option<String>)> = tx
            .prepare(
                "SELECT state, result_json FROM upload_sessions WHERE id = ?1",
            )?
            .query_row([sid], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .ok();

        let (state, cached_result) = match session {
            Some(s) => s,
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
            other => {
                anyhow::bail!("unexpected upload session state: {other}");
            }
        }

        let repo: String = tx
            .prepare("SELECT repo FROM upload_sessions WHERE id = ?1")?
            .query_row([sid], |r| r.get(0))?;

        // Apply ref updates one at a time. The whole thing is inside a
        // single IMMEDIATE transaction so the outcomes we record match the
        // state we leave on disk.
        let mut ref_results = Vec::with_capacity(updates.len());
        for u in updates {
            let success = apply_ref_update_tx(&tx, &repo, u)?;
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
                 SET state = 'committed', committed_at = ?2, result_json = ?3
                 WHERE id = ?1",
                rusqlite::params![sid, now, result_json],
            )?;
        } else {
            // Leave session in 'uploading' so the client can adjust (e.g.
            // rebase) and retry without losing its staged objects. The
            // sweeper still reclaims if the client never retries.
        }

        tx.commit()?;

        Ok(CommitSessionOutcome::Committed {
            ref_results,
            all_success,
        })
    }

    /// Return (id, repo) for sessions eligible for garbage collection:
    /// state = 'uploading' and expires_at <= cutoff, or state in
    /// ('failed','abandoned','committed') older than the cutoff.
    pub fn list_stale_upload_sessions(&self, cutoff_ts: i64) -> Result<Vec<(String, String)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, repo FROM upload_sessions
             WHERE (state = 'uploading' AND expires_at <= ?1)
                OR (state IN ('failed','abandoned','committed') AND
                    COALESCE(committed_at, created_at) <= ?1)",
        )?;
        let rows = stmt.query_map([cutoff_ts], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Delete a session row. Cascades to session_objects via FK. Caller is
    /// responsible for having already cleaned up the staging directory.
    pub fn delete_upload_session(&self, sid: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM upload_sessions WHERE id = ?1", [sid])?;
        Ok(())
    }

    // -- Agents (Phase 2 distributed runners) --

    pub fn create_agent_tables(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS agents (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT    NOT NULL UNIQUE,
                token_hash  TEXT    NOT NULL,
                labels_json TEXT    NOT NULL DEFAULT '[]',
                version     TEXT    NOT NULL DEFAULT '',
                os          TEXT    NOT NULL DEFAULT '',
                last_seen   INTEGER,
                created_at  INTEGER NOT NULL
            );
            -- Track which agent has claimed which run. Null claimed_by =
            -- the server's in-process engine (embedded mode).
            CREATE TABLE IF NOT EXISTS run_claims (
                run_id      INTEGER PRIMARY KEY,
                agent_id    INTEGER,
                claimed_at  INTEGER NOT NULL
            );
            ",
        )?;
        Ok(())
    }

    pub fn upsert_agent(
        &self,
        name: &str,
        token_hash: &str,
        labels_json: &str,
        version: &str,
        os: &str,
    ) -> Result<i64> {
        let conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO agents (name, token_hash, labels_json, version, os, last_seen, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(name) DO UPDATE SET
                labels_json = excluded.labels_json,
                version     = excluded.version,
                os          = excluded.os,
                last_seen   = excluded.last_seen",
            rusqlite::params![name, token_hash, labels_json, version, os, now],
        )?;
        let id: i64 = conn
            .prepare("SELECT id FROM agents WHERE name = ?1")?
            .query_row([name], |row| row.get(0))?;
        Ok(id)
    }

    pub fn get_agent_by_name(
        &self,
        name: &str,
    ) -> Result<Option<(i64, String, String)>> {
        let conn = self.conn()?;
        let result = conn
            .prepare(
                "SELECT id, token_hash, labels_json FROM agents WHERE name = ?1",
            )?
            .query_row([name], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .ok();
        Ok(result)
    }

    pub fn get_agent_by_id(
        &self,
        id: i64,
    ) -> Result<Option<(String, String, String)>> {
        let conn = self.conn()?;
        let result = conn
            .prepare(
                "SELECT name, token_hash, labels_json FROM agents WHERE id = ?1",
            )?
            .query_row([id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .ok();
        Ok(result)
    }

    pub fn touch_agent_last_seen(&self, id: i64) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE agents SET last_seen = ?1 WHERE id = ?2",
            rusqlite::params![chrono::Utc::now().timestamp(), id],
        )?;
        Ok(())
    }

    pub fn list_agents(
        &self,
    ) -> Result<Vec<(i64, String, String, i64, String, String)>> {
        // (id, name, labels_json, last_seen, version, os)
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, labels_json, COALESCE(last_seen, 0), version, os
             FROM agents ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn delete_agent(&self, id: i64) -> Result<bool> {
        let conn = self.conn()?;
        let n = conn.execute("DELETE FROM agents WHERE id = ?1", [id])?;
        Ok(n > 0)
    }

    /// Atomically claim the oldest queued run whose workflow doesn't
    /// require labels the agent is missing. Returns (run_id) on success
    /// or None when no work is available.
    pub fn claim_next_run(
        &self,
        agent_id: i64,
        _agent_labels: &[String],
    ) -> Result<Option<i64>> {
        // v1 label routing is permissive: if the run has no claim yet and
        // is queued, any agent can take it. Full label matching lands when
        // runs-on is parsed out of the workflow YAML in Phase 3.
        let mut conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        // BEGIN IMMEDIATE so the write lock is reserved up front. With a
        // deferred txn, the SELECT-then-INSERT pattern can hit
        // SQLITE_BUSY when another writer lands between the two
        // statements under pool contention.
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let candidate: Option<i64> = tx
            .prepare(
                "SELECT r.id FROM workflow_runs r
                 LEFT JOIN run_claims c ON c.run_id = r.id
                 WHERE r.status = 'queued' AND c.run_id IS NULL
                 ORDER BY r.created_at ASC LIMIT 1",
            )?
            .query_row([], |row| row.get(0))
            .ok();
        if let Some(run_id) = candidate {
            tx.execute(
                "INSERT INTO run_claims (run_id, agent_id, claimed_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![run_id, agent_id, now],
            )?;
            tx.execute(
                "UPDATE workflow_runs SET status = 'running', started_at = ?1 WHERE id = ?2",
                rusqlite::params![now, run_id],
            )?;
            tx.commit()?;
            Ok(Some(run_id))
        } else {
            Ok(None)
        }
    }

    /// Find runs claimed by agents whose `last_seen` is older than
    /// `cutoff_ts` (or never). Drops the claim and re-queues the run so
    /// another agent can pick it up. Returns the number of runs requeued.
    pub fn requeue_stale_runs(&self, cutoff_ts: i64) -> Result<usize> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Collect stale (run_id, agent_id) pairs first; we need the ids
        // to scope the workflow_runs reset to runs still in 'running'.
        let mut stmt = tx.prepare(
            "SELECT c.run_id, c.agent_id
             FROM run_claims c
             JOIN agents a ON a.id = c.agent_id
             WHERE COALESCE(a.last_seen, 0) < ?1",
        )?;
        let stale: Vec<(i64, i64)> = stmt
            .query_map([cutoff_ts], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        let mut n = 0usize;
        for (run_id, _agent_id) in &stale {
            tx.execute("DELETE FROM run_claims WHERE run_id = ?1", [run_id])?;
            // Only rewind runs that are still 'running'; if the agent
            // already reported success/failure we must not flip them back.
            let changed = tx.execute(
                "UPDATE workflow_runs
                 SET status = 'queued', started_at = NULL
                 WHERE id = ?1 AND status = 'running'",
                [run_id],
            )?;
            if changed > 0 {
                n += 1;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    pub fn get_run_claim_agent(&self, run_id: i64) -> Result<Option<i64>> {
        let conn = self.conn()?;
        let result = conn
            .prepare("SELECT agent_id FROM run_claims WHERE run_id = ?1")?
            .query_row([run_id], |row: &rusqlite::Row| row.get::<_, i64>(0))
            .ok();
        Ok(result)
    }

    // -- Secrets --

    pub fn create_secrets_tables(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS secrets (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                repo         TEXT    NOT NULL,
                key          TEXT    NOT NULL,
                nonce        BLOB    NOT NULL,
                ciphertext   BLOB    NOT NULL,
                created_at   INTEGER NOT NULL,
                updated_at   INTEGER NOT NULL,
                UNIQUE(repo, key)
            );
            ",
        )?;
        Ok(())
    }

    pub fn upsert_secret(
        &self,
        repo: &str,
        key: &str,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<()> {
        let conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO secrets (repo, key, nonce, ciphertext, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(repo, key) DO UPDATE SET
                nonce = excluded.nonce,
                ciphertext = excluded.ciphertext,
                updated_at = excluded.updated_at",
            rusqlite::params![repo, key, nonce, ciphertext, now],
        )?;
        Ok(())
    }

    pub fn get_secret(&self, repo: &str, key: &str) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        let conn = self.conn()?;
        let result = conn
            .prepare("SELECT nonce, ciphertext FROM secrets WHERE repo = ?1 AND key = ?2")?
            .query_row(rusqlite::params![repo, key], |row: &rusqlite::Row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .ok();
        Ok(result)
    }

    pub fn delete_secret(&self, repo: &str, key: &str) -> Result<bool> {
        let conn = self.conn()?;
        let n = conn.execute(
            "DELETE FROM secrets WHERE repo = ?1 AND key = ?2",
            rusqlite::params![repo, key],
        )?;
        Ok(n > 0)
    }

    pub fn list_secret_keys(
        &self,
        repo: &str,
    ) -> Result<Vec<crate::services::secrets::SecretMeta>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT repo, key, created_at, updated_at FROM secrets WHERE repo = ?1 ORDER BY key",
        )?;
        let rows = stmt.query_map([repo], |row: &rusqlite::Row| {
            Ok(crate::services::secrets::SecretMeta {
                repo: row.get(0)?,
                key: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    // -- Repos --

    pub fn list_repos(&self) -> Result<Vec<RepoRecord>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT name, description, created_at, visibility, default_branch FROM repos")?;
        let rows = stmt.query_map([], |row| {
            Ok(RepoRecord {
                name: row.get(0)?,
                description: row.get(1)?,
                created_at: row.get(2)?,
                visibility: row.get(3)?,
                default_branch: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Read the visibility flag for a single repo. Returns `None` if the
    /// repo doesn't exist. Used by the gRPC interceptor's read-path authz
    /// check to allow anonymous clones of public repos.
    pub fn get_repo_visibility(&self, name: &str) -> Result<Option<String>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT visibility FROM repos WHERE name = ?1")?;
        let result = stmt
            .query_row([name], |row| row.get::<_, String>(0))
            .ok();
        Ok(result)
    }

    /// Returns true if the repo is publicly readable (anonymous clone/pull
    /// allowed). Returns false for private repos and for repos that don't
    /// exist — the latter is fine because the read handler will fail later
    /// with NotFound.
    pub fn is_repo_public(&self, name: &str) -> bool {
        matches!(self.get_repo_visibility(name).ok().flatten().as_deref(), Some("public"))
    }

    /// Set the visibility of a repo. Returns true on success, false if the
    /// repo doesn't exist.
    pub fn set_repo_visibility(&self, name: &str, visibility: &str) -> Result<bool> {
        if visibility != "private" && visibility != "public" {
            anyhow::bail!("visibility must be 'private' or 'public'");
        }
        let conn = self.conn()?;
        let n = conn.execute(
            "UPDATE repos SET visibility = ?1 WHERE name = ?2",
            rusqlite::params![visibility, name],
        )?;
        Ok(n > 0)
    }

    pub fn create_repo(&self, name: &str, description: &str) -> Result<bool> {
        let conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        let affected = conn.execute(
            "INSERT OR IGNORE INTO repos (name, description, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![name, description, now],
        )?;
        Ok(affected > 0)
    }

    pub fn update_repo(&self, name: &str, new_name: &str, description: &str) -> Result<bool> {
        let mut conn = self.conn()?;

        // Check that the repo exists.
        let exists: bool = conn
            .prepare("SELECT COUNT(*) FROM repos WHERE name = ?1")?
            .query_row([name], |row| row.get::<_, i64>(0))
            .map(|c| c > 0)?;
        if !exists {
            return Ok(false);
        }

        let effective_name = if new_name.is_empty() { name } else { new_name };

        // If renaming, check that the new name is not already taken.
        if !new_name.is_empty() && new_name != name {
            let taken: bool = conn
                .prepare("SELECT COUNT(*) FROM repos WHERE name = ?1")?
                .query_row([new_name], |row| row.get::<_, i64>(0))
                .map(|c| c > 0)?;
            if taken {
                anyhow::bail!("repo '{}' already exists", new_name);
            }
        }

        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        tx.execute(
            "UPDATE repos SET name = ?1, description = ?2 WHERE name = ?3",
            rusqlite::params![effective_name, description, name],
        )?;

        // Update refs and locks tables if renamed.
        if !new_name.is_empty() && new_name != name {
            tx.execute(
                "UPDATE refs SET repo = ?1 WHERE repo = ?2",
                rusqlite::params![new_name, name],
            )?;
            tx.execute(
                "UPDATE locks SET repo = ?1 WHERE repo = ?2",
                rusqlite::params![new_name, name],
            )?;
        }

        tx.commit()?;
        Ok(true)
    }

    pub fn delete_repo(&self, name: &str) -> Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let affected = tx.execute("DELETE FROM repos WHERE name = ?1", [name])?;
        tx.execute("DELETE FROM refs WHERE repo = ?1", [name])?;
        tx.execute("DELETE FROM locks WHERE repo = ?1", [name])?;
        tx.commit()?;
        Ok(affected > 0)
    }

    // -- Refs --

    pub fn get_ref(&self, repo: &str, name: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT hash FROM refs WHERE repo = ?1 AND name = ?2")?;
        let result = stmt
            .query_row(rusqlite::params![repo, name], |row| row.get::<_, Vec<u8>>(0))
            .ok();
        Ok(result)
    }

    pub fn get_all_refs(&self, repo: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT name, hash FROM refs WHERE repo = ?1")?;
        let rows = stmt.query_map([repo], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Update a ref. Returns true if the row was changed.
    ///
    /// Three modes, picked by the caller:
    ///
    /// - `force = true` → unconditional `INSERT OR REPLACE`. Used by
    ///   `forge push --force` to publish a rewritten history. Skips both the
    ///   create-only and CAS checks below.
    /// - `old_hash` is all-zeros (and `force` is false) → create-only:
    ///   `INSERT OR IGNORE`, succeeds when the ref doesn't exist yet.
    /// - otherwise → atomic compare-and-swap: `UPDATE … WHERE hash = old_hash`,
    ///   succeeds only if the current ref still matches `old_hash`.
    pub fn update_ref(
        &self,
        repo: &str,
        name: &str,
        old_hash: &[u8],
        new_hash: &[u8],
        force: bool,
    ) -> Result<bool> {
        let conn = self.conn()?;

        if force {
            // Force overwrite — INSERT new row or replace existing one. We
            // don't return false on a no-op overwrite because callers want
            // success when the new_hash equals what's already there.
            let affected = conn.execute(
                "INSERT INTO refs (repo, name, hash) VALUES (?1, ?2, ?3)
                 ON CONFLICT(repo, name) DO UPDATE SET hash = excluded.hash",
                rusqlite::params![repo, name, new_hash],
            )?;
            return Ok(affected > 0);
        }

        let is_create = old_hash.iter().all(|&b| b == 0);

        let affected = if is_create {
            // Expect ref to not exist — INSERT only if absent
            conn.execute(
                "INSERT OR IGNORE INTO refs (repo, name, hash) VALUES (?1, ?2, ?3)",
                rusqlite::params![repo, name, new_hash],
            )?
        } else {
            // Atomic CAS: update only if current hash matches old_hash
            conn.execute(
                "UPDATE refs SET hash = ?1 WHERE repo = ?2 AND name = ?3 AND hash = ?4",
                rusqlite::params![new_hash, repo, name, old_hash],
            )?
        };

        Ok(affected > 0)
    }

    // -- Locks --

    /// Try to acquire a lock. Returns Ok(true) if acquired, Ok(false) with existing lock info if denied.
    pub fn acquire_lock(
        &self,
        repo: &str,
        path: &str,
        owner: &str,
        workspace_id: &str,
        reason: &str,
    ) -> Result<std::result::Result<(), LockInfo>> {
        let conn = self.conn()?;

        // Check if already locked.
        if let Ok(lock) = conn.prepare("SELECT owner, workspace_id, created_at, reason FROM locks WHERE repo = ?1 AND path = ?2")?
            .query_row(rusqlite::params![repo, path], |row| {
                Ok(LockInfo {
                    path: path.to_string(),
                    owner: row.get(0)?,
                    workspace_id: row.get(1)?,
                    created_at: row.get(2)?,
                    reason: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                })
            })
        {
            if lock.owner == owner {
                return Ok(Ok(())); // Already locked by same owner.
            }
            return Ok(Err(lock));
        }

        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO locks (repo, path, owner, workspace_id, created_at, reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![repo, path, owner, workspace_id, now, reason],
        )?;

        Ok(Ok(()))
    }

    pub fn release_lock(&self, repo: &str, path: &str, owner: &str, force: bool) -> Result<bool> {
        let conn = self.conn()?;
        let affected = if force {
            conn.execute("DELETE FROM locks WHERE repo = ?1 AND path = ?2", rusqlite::params![repo, path])?
        } else {
            conn.execute(
                "DELETE FROM locks WHERE repo = ?1 AND path = ?2 AND owner = ?3",
                rusqlite::params![repo, path, owner],
            )?
        };
        Ok(affected > 0)
    }

    pub fn list_locks(&self, repo: &str, path_prefix: &str, owner_filter: &str) -> Result<Vec<LockInfo>> {
        let conn = self.conn()?;
        let mut locks = Vec::new();

        let prefix_pattern = if path_prefix.is_empty() {
            "%".to_string()
        } else {
            format!("{}%", path_prefix)
        };
        let owner_pattern = if owner_filter.is_empty() {
            "%".to_string()
        } else {
            owner_filter.to_string()
        };

        let mut stmt = conn.prepare(
            "SELECT path, owner, workspace_id, created_at, reason FROM locks WHERE repo = ?1 AND path LIKE ?2 AND owner LIKE ?3 LIMIT 10000"
        )?;

        let rows = stmt.query_map(rusqlite::params![repo, prefix_pattern, owner_pattern], |row| {
            Ok(LockInfo {
                path: row.get(0)?,
                owner: row.get(1)?,
                workspace_id: row.get(2)?,
                created_at: row.get(3)?,
                reason: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            })
        })?;

        for row in rows {
            locks.push(row?);
        }

        Ok(locks)
    }

    // -- Issues --

    pub fn list_issues(&self, repo: &str, status: &str, limit: i32, offset: i32) -> Result<(Vec<IssueRecord>, i32, i32, i32)> {
        let conn = self.conn()?;
        let lim = if limit <= 0 { 50 } else { limit };

        let open_count: i32 = conn
            .prepare("SELECT COUNT(*) FROM issues WHERE repo = ?1 AND status = 'open'")?
            .query_row([repo], |row| row.get(0))?;
        let closed_count: i32 = conn
            .prepare("SELECT COUNT(*) FROM issues WHERE repo = ?1 AND status = 'closed'")?
            .query_row([repo], |row| row.get(0))?;

        let (query, total) = if status.is_empty() {
            ("SELECT id, repo, title, body, author, status, labels, created_at, updated_at, comment_count, assignee FROM issues WHERE repo = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3".to_string(),
             open_count + closed_count)
        } else {
            ("SELECT id, repo, title, body, author, status, labels, created_at, updated_at, comment_count, assignee FROM issues WHERE repo = ?1 AND status = ?4 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3".to_string(),
             if status == "open" { open_count } else { closed_count })
        };

        let mut stmt = conn.prepare(&query)?;
        let rows = if status.is_empty() {
            stmt.query_map(rusqlite::params![repo, lim, offset], Self::map_issue)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(rusqlite::params![repo, lim, offset, status], Self::map_issue)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };

        Ok((rows, total, open_count, closed_count))
    }

    pub fn create_issue(&self, repo: &str, title: &str, body: &str, author: &str, labels: &str) -> Result<i64> {
        let conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO issues (repo, title, body, author, status, labels, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6, ?7)",
            rusqlite::params![repo, title, body, author, labels, now, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Get a single issue by ID.
    pub fn get_issue(&self, id: i64) -> Result<Option<IssueRecord>> {
        let conn = self.conn()?;
        let result = conn
            .prepare("SELECT id, repo, title, body, author, status, labels, created_at, updated_at, comment_count, assignee FROM issues WHERE id = ?1")?
            .query_row([id], Self::map_issue)
            .ok();
        Ok(result)
    }

    /// Partial update: empty strings mean "keep current value".
    pub fn update_issue(&self, id: i64, title: &str, body: &str, status: &str, labels: &str, assignee: &str) -> Result<bool> {
        let current = self.get_issue(id)?;
        let current = match current {
            Some(c) => c,
            None => return Ok(false),
        };

        let conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        let new_title = if title.is_empty() { &current.title } else { title };
        let new_body = if body.is_empty() { &current.body } else { body };
        let new_status = if status.is_empty() { &current.status } else { status };
        let new_labels = if labels.is_empty() { &current.labels } else { labels };
        let new_assignee = if assignee.is_empty() { &current.assignee } else { assignee };

        let affected = conn.execute(
            "UPDATE issues SET title = ?1, body = ?2, status = ?3, labels = ?4, assignee = ?5, updated_at = ?6 WHERE id = ?7",
            rusqlite::params![new_title, new_body, new_status, new_labels, new_assignee, now, id],
        )?;
        Ok(affected > 0)
    }

    fn map_issue(row: &rusqlite::Row<'_>) -> rusqlite::Result<IssueRecord> {
        Ok(IssueRecord {
            id: row.get(0)?,
            repo: row.get(1)?,
            title: row.get(2)?,
            body: row.get(3)?,
            author: row.get(4)?,
            status: row.get(5)?,
            labels: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
            comment_count: row.get(9)?,
            assignee: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
        })
    }

    // -- Pull Requests --

    pub fn list_pull_requests(&self, repo: &str, status: &str, limit: i32, offset: i32) -> Result<(Vec<PullRequestRecord>, i32, i32, i32)> {
        let conn = self.conn()?;
        let lim = if limit <= 0 { 50 } else { limit };

        let open_count: i32 = conn
            .prepare("SELECT COUNT(*) FROM pull_requests WHERE repo = ?1 AND status = 'open'")?
            .query_row([repo], |row| row.get(0))?;
        let closed_count: i32 = conn
            .prepare("SELECT COUNT(*) FROM pull_requests WHERE repo = ?1 AND (status = 'closed' OR status = 'merged')")?
            .query_row([repo], |row| row.get(0))?;

        let (query, total) = if status.is_empty() {
            ("SELECT id, repo, title, body, author, status, source_branch, target_branch, labels, created_at, updated_at, comment_count, assignee FROM pull_requests WHERE repo = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3".to_string(),
             open_count + closed_count)
        } else {
            ("SELECT id, repo, title, body, author, status, source_branch, target_branch, labels, created_at, updated_at, comment_count, assignee FROM pull_requests WHERE repo = ?1 AND status = ?4 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3".to_string(),
             if status == "open" { open_count } else { closed_count })
        };

        let mut stmt = conn.prepare(&query)?;
        let rows = if status.is_empty() {
            stmt.query_map(rusqlite::params![repo, lim, offset], Self::map_pr)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(rusqlite::params![repo, lim, offset, status], Self::map_pr)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };

        Ok((rows, total, open_count, closed_count))
    }

    pub fn create_pull_request(&self, repo: &str, title: &str, body: &str, author: &str, source_branch: &str, target_branch: &str, labels: &str) -> Result<i64> {
        let conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO pull_requests (repo, title, body, author, status, source_branch, target_branch, labels, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![repo, title, body, author, source_branch, target_branch, labels, now, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Get a single pull request by ID.
    pub fn get_pull_request(&self, id: i64) -> Result<Option<PullRequestRecord>> {
        let conn = self.conn()?;
        let result = conn
            .prepare("SELECT id, repo, title, body, author, status, source_branch, target_branch, labels, created_at, updated_at, comment_count, assignee FROM pull_requests WHERE id = ?1")?
            .query_row([id], Self::map_pr)
            .ok();
        Ok(result)
    }

    /// Partial update: empty strings mean "keep current value".
    pub fn update_pull_request(&self, id: i64, title: &str, body: &str, status: &str, labels: &str, assignee: &str) -> Result<bool> {
        let current = self.get_pull_request(id)?;
        let current = match current {
            Some(c) => c,
            None => return Ok(false),
        };

        let conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        let new_title = if title.is_empty() { &current.title } else { title };
        let new_body = if body.is_empty() { &current.body } else { body };
        let new_status = if status.is_empty() { &current.status } else { status };
        let new_labels = if labels.is_empty() { &current.labels } else { labels };
        let new_assignee = if assignee.is_empty() { &current.assignee } else { assignee };

        let affected = conn.execute(
            "UPDATE pull_requests SET title = ?1, body = ?2, status = ?3, labels = ?4, assignee = ?5, updated_at = ?6 WHERE id = ?7",
            rusqlite::params![new_title, new_body, new_status, new_labels, new_assignee, now, id],
        )?;
        Ok(affected > 0)
    }

    // -- Default branch --

    pub fn get_default_branch(&self, repo: &str) -> Result<String> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT default_branch FROM repos WHERE name = ?1")?;
        let result = stmt
            .query_row([repo], |row| row.get::<_, String>(0))
            .ok()
            .unwrap_or_default();
        Ok(result)
    }

    pub fn set_default_branch(&self, repo: &str, branch: &str) -> Result<bool> {
        let conn = self.conn()?;
        let n = conn.execute(
            "UPDATE repos SET default_branch = ?1 WHERE name = ?2",
            rusqlite::params![branch, repo],
        )?;
        Ok(n > 0)
    }

    // -- Comments --

    pub fn list_comments(&self, repo: &str, issue_id: i64, kind: &str) -> Result<Vec<CommentRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, repo, issue_id, kind, author, body, created_at, updated_at
             FROM comments WHERE repo = ?1 AND issue_id = ?2 AND kind = ?3
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![repo, issue_id, kind], |row| {
            Ok(CommentRecord {
                id: row.get(0)?,
                repo: row.get(1)?,
                issue_id: row.get(2)?,
                kind: row.get(3)?,
                author: row.get(4)?,
                body: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_comment(&self, repo: &str, issue_id: i64, kind: &str, author: &str, body: &str) -> Result<i64> {
        let conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO comments (repo, issue_id, kind, author, body, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![repo, issue_id, kind, author, body, now, now],
        )?;
        let id = conn.last_insert_rowid();
        // Increment comment_count on the parent
        let table = if kind == "pull_request" { "pull_requests" } else { "issues" };
        conn.execute(
            &format!("UPDATE {table} SET comment_count = comment_count + 1 WHERE id = ?1"),
            [issue_id],
        )?;
        Ok(id)
    }

    pub fn update_comment(&self, id: i64, body: &str) -> Result<bool> {
        let conn = self.conn()?;
        let now = chrono::Utc::now().timestamp();
        let n = conn.execute(
            "UPDATE comments SET body = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![body, now, id],
        )?;
        Ok(n > 0)
    }

    pub fn delete_comment(&self, id: i64) -> Result<bool> {
        let conn = self.conn()?;
        // Get the comment first to decrement the parent's count
        let comment: Option<(String, i64, String)> = conn
            .prepare("SELECT repo, issue_id, kind FROM comments WHERE id = ?1")?
            .query_row([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .ok();
        let n = conn.execute("DELETE FROM comments WHERE id = ?1", [id])?;
        if n > 0 {
            if let Some((_repo, issue_id, kind)) = comment {
                let table = if kind == "pull_request" { "pull_requests" } else { "issues" };
                conn.execute(
                    &format!("UPDATE {table} SET comment_count = CASE WHEN comment_count > 0 THEN comment_count - 1 ELSE 0 END WHERE id = ?1"),
                    [issue_id],
                )?;
            }
        }
        Ok(n > 0)
    }

    pub fn get_comment(&self, id: i64) -> Result<Option<CommentRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, repo, issue_id, kind, author, body, created_at, updated_at FROM comments WHERE id = ?1",
        )?;
        let result = stmt
            .query_row([id], |row| {
                Ok(CommentRecord {
                    id: row.get(0)?,
                    repo: row.get(1)?,
                    issue_id: row.get(2)?,
                    kind: row.get(3)?,
                    author: row.get(4)?,
                    body: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .ok();
        Ok(result)
    }

    fn map_pr(row: &rusqlite::Row<'_>) -> rusqlite::Result<PullRequestRecord> {
        Ok(PullRequestRecord {
            id: row.get(0)?,
            repo: row.get(1)?,
            title: row.get(2)?,
            body: row.get(3)?,
            author: row.get(4)?,
            status: row.get(5)?,
            source_branch: row.get(6)?,
            target_branch: row.get(7)?,
            labels: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
            comment_count: row.get(11)?,
            assignee: row.get::<_, Option<String>>(12)?.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct IssueRecord {
    pub id: i64,
    pub repo: String,
    pub title: String,
    pub body: String,
    pub author: String,
    pub status: String,
    pub labels: String,
    pub assignee: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub comment_count: i32,
}

#[derive(Debug, Clone)]
pub struct PullRequestRecord {
    pub id: i64,
    pub repo: String,
    pub title: String,
    pub body: String,
    pub author: String,
    pub status: String,
    pub source_branch: String,
    pub target_branch: String,
    pub labels: String,
    pub assignee: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub comment_count: i32,
}

#[derive(Debug, Clone)]
pub struct LockInfo {
    pub path: String,
    pub owner: String,
    pub workspace_id: String,
    pub created_at: i64,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct UploadSessionRecord {
    pub id: String,
    pub repo: String,
    pub user_id: Option<i64>,
    pub state: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub committed_at: Option<i64>,
    pub result_json: Option<String>,
    pub failure: Option<String>,
}

/// One ref update inside an atomic CommitPush. Borrowed to avoid cloning
/// hash buffers on the hot path.
#[derive(Debug, Clone)]
pub struct RefUpdateSpec<'a> {
    pub ref_name: &'a str,
    pub old_hash: &'a [u8],
    pub new_hash: &'a [u8],
    pub force: bool,
}

/// Per-ref outcome captured inside the commit transaction.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefUpdateOutcome {
    pub ref_name: String,
    pub success: bool,
    pub error: String,
}

/// High-level result of `commit_upload_session`.
#[derive(Debug)]
pub enum CommitSessionOutcome {
    /// Session id is not known to the server.
    Unknown,
    /// First-time commit completed. `all_success` is false when one or
    /// more refs failed their CAS (the session is left in 'uploading' so
    /// the client can rebase + retry without re-uploading objects).
    Committed {
        ref_results: Vec<RefUpdateOutcome>,
        all_success: bool,
    },
    /// Retry against an already-committed session. The client should
    /// decode `result_json` and return the same response it would have
    /// gotten the first time.
    AlreadyCommitted { result_json: String },
    /// Retry against a session that was already failed. Client surfaces
    /// the prior outcome to the user (the session cannot be revived —
    /// start a new push).
    TerminallyFailed {
        reason: String,
        result_json: String,
    },
}

/// Apply one ref update using the same semantics as [`MetadataDb::update_ref`]
/// but inside an existing transaction. Kept private — callers go through
/// [`MetadataDb::commit_upload_session`].
fn apply_ref_update_tx(
    tx: &rusqlite::Transaction<'_>,
    repo: &str,
    u: &RefUpdateSpec<'_>,
) -> Result<bool> {
    if u.force {
        let affected = tx.execute(
            "INSERT INTO refs (repo, name, hash) VALUES (?1, ?2, ?3)
             ON CONFLICT(repo, name) DO UPDATE SET hash = excluded.hash",
            rusqlite::params![repo, u.ref_name, u.new_hash],
        )?;
        return Ok(affected > 0);
    }

    let is_create = u.old_hash.iter().all(|&b| b == 0);

    let affected = if is_create {
        tx.execute(
            "INSERT OR IGNORE INTO refs (repo, name, hash) VALUES (?1, ?2, ?3)",
            rusqlite::params![repo, u.ref_name, u.new_hash],
        )?
    } else {
        tx.execute(
            "UPDATE refs SET hash = ?1 WHERE repo = ?2 AND name = ?3 AND hash = ?4",
            rusqlite::params![u.new_hash, repo, u.ref_name, u.old_hash],
        )?
    };

    Ok(affected > 0)
}

#[derive(Debug, Clone)]
pub struct RepoRecord {
    pub name: String,
    pub description: String,
    pub created_at: i64,
    pub visibility: String,
    pub default_branch: String,
}

#[derive(Debug, Clone)]
pub struct CommentRecord {
    pub id: i64,
    pub repo: String,
    pub issue_id: i64,
    pub kind: String,
    pub author: String,
    pub body: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl crate::storage::backend::MetadataBackend for MetadataDb {
    fn list_repos(&self) -> Result<Vec<RepoRecord>> {
        MetadataDb::list_repos(self)
    }
    fn get_repo_visibility(&self, name: &str) -> Result<Option<String>> {
        MetadataDb::get_repo_visibility(self, name)
    }
    fn is_repo_public(&self, name: &str) -> bool {
        MetadataDb::is_repo_public(self, name)
    }
    fn set_repo_visibility(&self, name: &str, visibility: &str) -> Result<bool> {
        MetadataDb::set_repo_visibility(self, name, visibility)
    }
    fn create_repo(&self, name: &str, description: &str) -> Result<bool> {
        MetadataDb::create_repo(self, name, description)
    }
    fn update_repo(&self, name: &str, new_name: &str, description: &str) -> Result<bool> {
        MetadataDb::update_repo(self, name, new_name, description)
    }
    fn delete_repo(&self, name: &str) -> Result<bool> {
        MetadataDb::delete_repo(self, name)
    }

    fn get_ref(&self, repo: &str, name: &str) -> Result<Option<Vec<u8>>> {
        MetadataDb::get_ref(self, repo, name)
    }
    fn get_all_refs(&self, repo: &str) -> Result<Vec<(String, Vec<u8>)>> {
        MetadataDb::get_all_refs(self, repo)
    }
    fn update_ref(
        &self,
        repo: &str,
        name: &str,
        old_hash: &[u8],
        new_hash: &[u8],
        force: bool,
    ) -> Result<bool> {
        MetadataDb::update_ref(self, repo, name, old_hash, new_hash, force)
    }

    fn acquire_lock(
        &self,
        repo: &str,
        path: &str,
        owner: &str,
        workspace_id: &str,
        reason: &str,
    ) -> Result<std::result::Result<(), LockInfo>> {
        MetadataDb::acquire_lock(self, repo, path, owner, workspace_id, reason)
    }
    fn release_lock(&self, repo: &str, path: &str, owner: &str, force: bool) -> Result<bool> {
        MetadataDb::release_lock(self, repo, path, owner, force)
    }
    fn list_locks(
        &self,
        repo: &str,
        path_prefix: &str,
        owner_filter: &str,
    ) -> Result<Vec<LockInfo>> {
        MetadataDb::list_locks(self, repo, path_prefix, owner_filter)
    }

    fn create_upload_session(
        &self,
        sid: &str,
        repo: &str,
        user_id: Option<i64>,
        ttl_seconds: i64,
    ) -> Result<()> {
        MetadataDb::create_upload_session(self, sid, repo, user_id, ttl_seconds)
    }
    fn record_session_object(&self, sid: &str, hash: &[u8], size: i64) -> Result<()> {
        MetadataDb::record_session_object(self, sid, hash, size)
    }
    fn get_upload_session(&self, sid: &str) -> Result<Option<UploadSessionRecord>> {
        MetadataDb::get_upload_session(self, sid)
    }
    fn list_session_object_hashes(&self, sid: &str) -> Result<Vec<Vec<u8>>> {
        MetadataDb::list_session_object_hashes(self, sid)
    }
    fn list_session_objects_with_sizes(&self, sid: &str) -> Result<Vec<(Vec<u8>, i64)>> {
        MetadataDb::list_session_objects_with_sizes(self, sid)
    }
    fn fail_upload_session(&self, sid: &str, reason: &str, result_json: &str) -> Result<()> {
        MetadataDb::fail_upload_session(self, sid, reason, result_json)
    }
    fn commit_upload_session(
        &self,
        sid: &str,
        updates: &[RefUpdateSpec<'_>],
    ) -> Result<CommitSessionOutcome> {
        MetadataDb::commit_upload_session(self, sid, updates)
    }
    fn list_stale_upload_sessions(&self, cutoff_ts: i64) -> Result<Vec<(String, String)>> {
        MetadataDb::list_stale_upload_sessions(self, cutoff_ts)
    }
    fn delete_upload_session(&self, sid: &str) -> Result<()> {
        MetadataDb::delete_upload_session(self, sid)
    }

    fn current_schema_version(&self) -> Result<i64> {
        MetadataDb::current_schema_version(self)
    }

    fn apply_pending_migrations(&self) -> Result<usize> {
        let current = MetadataDb::current_schema_version(self)?;
        let mut conn = self.conn()?;
        crate::storage::migrations::apply_pending(
            &mut conn,
            current,
            crate::storage::migrations::SQLITE_MIGRATIONS,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_db() -> (TempDir, MetadataDb) {
        let tmp = TempDir::new().unwrap();
        let db = MetadataDb::open(&tmp.path().join("forge.db")).unwrap();
        // The repos table has a NOT NULL FK relationship in the refs CHECK
        // so we register the repo first.
        db.create_repo("alice/forcetest", "").unwrap();
        (tmp, db)
    }

    const ZERO: [u8; 32] = [0u8; 32];

    fn h(byte: u8) -> Vec<u8> {
        vec![byte; 32]
    }

    // ── update_ref ───────────────────────────────────────────────────────────

    #[test]
    fn create_path_inserts_when_absent() {
        let (_tmp, db) = fresh_db();
        // old_hash = zeros + force = false → INSERT OR IGNORE
        let ok = db
            .update_ref("alice/forcetest", "refs/heads/main", &ZERO, &h(0xAA), false)
            .unwrap();
        assert!(ok);
        let stored = db
            .get_ref("alice/forcetest", "refs/heads/main")
            .unwrap()
            .unwrap();
        assert_eq!(stored, h(0xAA));
    }

    #[test]
    fn create_path_no_op_when_already_present() {
        let (_tmp, db) = fresh_db();
        db.update_ref("alice/forcetest", "refs/heads/main", &ZERO, &h(0xAA), false)
            .unwrap();
        // Second create returns false because INSERT OR IGNORE skips.
        let ok = db
            .update_ref("alice/forcetest", "refs/heads/main", &ZERO, &h(0xBB), false)
            .unwrap();
        assert!(!ok);
        // The original hash is preserved.
        assert_eq!(
            db.get_ref("alice/forcetest", "refs/heads/main").unwrap().unwrap(),
            h(0xAA)
        );
    }

    #[test]
    fn cas_path_succeeds_when_old_hash_matches() {
        let (_tmp, db) = fresh_db();
        db.update_ref("alice/forcetest", "refs/heads/main", &ZERO, &h(0xAA), false)
            .unwrap();
        // CAS update with the right old_hash succeeds.
        let ok = db
            .update_ref("alice/forcetest", "refs/heads/main", &h(0xAA), &h(0xBB), false)
            .unwrap();
        assert!(ok);
        assert_eq!(
            db.get_ref("alice/forcetest", "refs/heads/main").unwrap().unwrap(),
            h(0xBB)
        );
    }

    #[test]
    fn cas_path_fails_when_old_hash_stale() {
        let (_tmp, db) = fresh_db();
        db.update_ref("alice/forcetest", "refs/heads/main", &ZERO, &h(0xAA), false)
            .unwrap();
        // Someone else moved the ref to 0xBB
        db.update_ref("alice/forcetest", "refs/heads/main", &h(0xAA), &h(0xBB), false)
            .unwrap();
        // We try to update assuming it's still 0xAA — must fail.
        let ok = db
            .update_ref("alice/forcetest", "refs/heads/main", &h(0xAA), &h(0xCC), false)
            .unwrap();
        assert!(!ok);
        // And the existing ref is untouched.
        assert_eq!(
            db.get_ref("alice/forcetest", "refs/heads/main").unwrap().unwrap(),
            h(0xBB)
        );
    }

    #[test]
    fn force_overwrites_existing_ref() {
        let (_tmp, db) = fresh_db();
        db.update_ref("alice/forcetest", "refs/heads/main", &ZERO, &h(0xAA), false)
            .unwrap();
        // Force-push to a totally unrelated hash, with a stale old_hash to
        // prove force bypasses the CAS check.
        let ok = db
            .update_ref("alice/forcetest", "refs/heads/main", &h(0xDE), &h(0xCC), true)
            .unwrap();
        assert!(ok);
        assert_eq!(
            db.get_ref("alice/forcetest", "refs/heads/main").unwrap().unwrap(),
            h(0xCC)
        );
    }

    #[test]
    fn force_creates_ref_when_absent() {
        let (_tmp, db) = fresh_db();
        // Force on a brand-new ref should also work — it inserts the row.
        let ok = db
            .update_ref("alice/forcetest", "refs/heads/dev", &ZERO, &h(0xEE), true)
            .unwrap();
        assert!(ok);
        assert_eq!(
            db.get_ref("alice/forcetest", "refs/heads/dev").unwrap().unwrap(),
            h(0xEE)
        );
    }

    #[test]
    fn force_with_same_hash_is_a_noop_but_reports_success() {
        let (_tmp, db) = fresh_db();
        db.update_ref("alice/forcetest", "refs/heads/main", &ZERO, &h(0xAA), false)
            .unwrap();
        // Force-pushing the same hash should be a clean no-op (UPSERT
        // touches the row, affected = 1).
        let ok = db
            .update_ref("alice/forcetest", "refs/heads/main", &ZERO, &h(0xAA), true)
            .unwrap();
        assert!(ok);
        assert_eq!(
            db.get_ref("alice/forcetest", "refs/heads/main").unwrap().unwrap(),
            h(0xAA)
        );
    }

    // ── upload sessions (Phase 1 atomic push) ───────────────────────────

    #[test]
    fn session_create_is_idempotent() {
        let (_tmp, db) = fresh_db();
        // First create — inserts.
        db.create_upload_session("sid-1", "alice/forcetest", Some(42), 60).unwrap();
        // Second call is a silent no-op via INSERT OR IGNORE.
        db.create_upload_session("sid-1", "alice/forcetest", Some(99), 60).unwrap();

        let rec = db.get_upload_session("sid-1").unwrap().unwrap();
        assert_eq!(rec.id, "sid-1");
        assert_eq!(rec.state, "uploading");
        // First insertion wins — user_id stays 42, not 99.
        assert_eq!(rec.user_id, Some(42));
    }

    #[test]
    fn session_objects_dedup() {
        let (_tmp, db) = fresh_db();
        db.create_upload_session("sid-1", "alice/forcetest", None, 60).unwrap();
        db.record_session_object("sid-1", &h(0x11), 100).unwrap();
        // Duplicate hash must not blow up; INSERT OR IGNORE handles it.
        db.record_session_object("sid-1", &h(0x11), 100).unwrap();
        db.record_session_object("sid-1", &h(0x22), 200).unwrap();

        let hashes = db.list_session_object_hashes("sid-1").unwrap();
        assert_eq!(hashes.len(), 2, "dup hash should not produce dup row");
        assert!(hashes.iter().any(|h| h == &vec![0x11u8; 32]));
        assert!(hashes.iter().any(|h| h == &vec![0x22u8; 32]));
    }

    #[test]
    fn commit_session_applies_ref_and_marks_committed() {
        let (_tmp, db) = fresh_db();
        db.create_upload_session("sid-1", "alice/forcetest", None, 60).unwrap();

        let update = RefUpdateSpec {
            ref_name: "refs/heads/main",
            old_hash: &ZERO,
            new_hash: &h(0xAA),
            force: false,
        };
        let outcome = db.commit_upload_session("sid-1", &[update]).unwrap();
        match outcome {
            CommitSessionOutcome::Committed {
                ref_results,
                all_success,
            } => {
                assert!(all_success);
                assert_eq!(ref_results.len(), 1);
                assert!(ref_results[0].success);
            }
            other => panic!("expected Committed, got {other:?}"),
        }
        // Ref is in place.
        assert_eq!(
            db.get_ref("alice/forcetest", "refs/heads/main").unwrap().unwrap(),
            h(0xAA)
        );
        // Session marked committed.
        let rec = db.get_upload_session("sid-1").unwrap().unwrap();
        assert_eq!(rec.state, "committed");
        assert!(rec.committed_at.is_some());
    }

    #[test]
    fn commit_session_is_idempotent_on_retry() {
        let (_tmp, db) = fresh_db();
        db.create_upload_session("sid-1", "alice/forcetest", None, 60).unwrap();

        let update = RefUpdateSpec {
            ref_name: "refs/heads/main",
            old_hash: &ZERO,
            new_hash: &h(0xAA),
            force: false,
        };

        // First commit moves the ref and marks session committed.
        let first = db.commit_upload_session("sid-1", &[update.clone()]).unwrap();
        assert!(matches!(first, CommitSessionOutcome::Committed { .. }));

        // Retry against the same session returns AlreadyCommitted WITHOUT
        // re-applying the ref. This is what makes a CommitPush retry safe
        // when the client loses its connection between the stream and the
        // commit reply.
        let retry = db.commit_upload_session("sid-1", &[update]).unwrap();
        match retry {
            CommitSessionOutcome::AlreadyCommitted { result_json } => {
                assert!(result_json.contains("refs/heads/main"));
            }
            other => panic!("expected AlreadyCommitted, got {other:?}"),
        }
    }

    #[test]
    fn commit_session_cas_failure_leaves_session_uploading() {
        let (_tmp, db) = fresh_db();
        // Start with main@0xAA already there (someone else won the race).
        db.update_ref("alice/forcetest", "refs/heads/main", &ZERO, &h(0xAA), false)
            .unwrap();

        db.create_upload_session("sid-1", "alice/forcetest", None, 60).unwrap();

        // Our push thinks main was at zero — a stale view.
        let stale = RefUpdateSpec {
            ref_name: "refs/heads/main",
            old_hash: &ZERO,
            new_hash: &h(0xBB),
            force: false,
        };
        let outcome = db.commit_upload_session("sid-1", &[stale]).unwrap();
        match outcome {
            CommitSessionOutcome::Committed {
                ref_results,
                all_success,
            } => {
                assert!(!all_success);
                assert_eq!(ref_results.len(), 1);
                assert!(!ref_results[0].success);
            }
            other => panic!("expected Committed with failure, got {other:?}"),
        }

        // Session is NOT marked committed so the client can re-plan (pull,
        // rebase, retry) without re-uploading objects.
        let rec = db.get_upload_session("sid-1").unwrap().unwrap();
        assert_eq!(rec.state, "uploading");

        // Ref is unchanged.
        assert_eq!(
            db.get_ref("alice/forcetest", "refs/heads/main").unwrap().unwrap(),
            h(0xAA)
        );
    }

    #[test]
    fn commit_session_rejects_unknown_id() {
        let (_tmp, db) = fresh_db();
        let update = RefUpdateSpec {
            ref_name: "refs/heads/main",
            old_hash: &ZERO,
            new_hash: &h(0xAA),
            force: false,
        };
        let outcome = db.commit_upload_session("does-not-exist", &[update]).unwrap();
        assert!(matches!(outcome, CommitSessionOutcome::Unknown));
    }

    #[test]
    fn commit_session_surfaces_prior_failure() {
        let (_tmp, db) = fresh_db();
        db.create_upload_session("sid-1", "alice/forcetest", None, 60).unwrap();
        db.fail_upload_session("sid-1", "lock_conflict", "[]").unwrap();

        let update = RefUpdateSpec {
            ref_name: "refs/heads/main",
            old_hash: &ZERO,
            new_hash: &h(0xAA),
            force: false,
        };
        let outcome = db.commit_upload_session("sid-1", &[update]).unwrap();
        match outcome {
            CommitSessionOutcome::TerminallyFailed { reason, .. } => {
                assert_eq!(reason, "failed");
            }
            other => panic!("expected TerminallyFailed, got {other:?}"),
        }
    }

    #[test]
    fn schema_version_records_baseline_then_migrations() {
        let (_tmp, db) = fresh_db();
        // Baseline is 1; the runner_bootstrap_check migration bumps to
        // at least 2. Future phases may push this higher — assert ≥ 2
        // rather than == 2 so this test doesn't need touching every
        // time a new migration lands.
        let v = db.current_schema_version().unwrap();
        assert!(
            v >= 2,
            "expected migrations to advance schema_version past baseline, got {v}"
        );
    }

    #[test]
    fn migration_runner_is_idempotent_across_opens() {
        // Open, close, re-open against the same DB file — the second
        // open() must be a no-op from the runner's perspective.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("forge.db");
        let v1 = {
            let db = MetadataDb::open(&path).unwrap();
            db.current_schema_version().unwrap()
        };
        let v2 = {
            let db = MetadataDb::open(&path).unwrap();
            db.current_schema_version().unwrap()
        };
        assert_eq!(v1, v2, "re-open must not change schema_version");

        // And the runner_bootstrap_check sentinel row is still there
        // exactly once (confirming ON CONFLICT DO NOTHING held).
        let db = MetadataDb::open(&path).unwrap();
        let conn = db.conn().unwrap();
        let count: i64 = conn
            .prepare("SELECT COUNT(*) FROM schema_runner_check")
            .unwrap()
            .query_row([], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "bootstrap-check row must be singleton");
    }

    #[test]
    fn migration_apply_pending_detects_ascending_only() {
        // The runner's debug-only invariant: migration versions must
        // ascend strictly. Constructing an out-of-order list and
        // passing it through apply_pending would trip the assert —
        // we verify the same condition logically here so Postgres (or
        // future maintainers) can't accidentally land a regression.
        use crate::storage::migrations::SQLITE_MIGRATIONS;
        let mut prev: i64 = 0;
        for m in SQLITE_MIGRATIONS {
            assert!(
                m.version > prev,
                "migration {} must have a higher version than {}",
                m.name,
                prev
            );
            prev = m.version;
        }
    }

    /// Regression guard for Phase 2a's core change: with the single
    /// Mutex gone, concurrent readers + writers must interleave instead
    /// of serialising. This test fails fast against the pre-Phase-2a
    /// design because every lock contention would be waited on
    /// sequentially; the pooled design completes in sub-second wall
    /// time on a laptop.
    #[test]
    fn concurrent_reads_and_writes_do_not_deadlock() {
        use std::sync::Arc;
        use std::thread;

        let tmp = TempDir::new().unwrap();
        let db = Arc::new(MetadataDb::open(&tmp.path().join("forge.db")).unwrap());
        db.create_repo("alice/pool", "").unwrap();
        // Seed a few refs so readers have something to scan.
        for b in 0u8..8 {
            db.update_ref(
                "alice/pool",
                &format!("refs/heads/b{b}"),
                &ZERO,
                &h(b),
                false,
            )
            .unwrap();
        }

        let start = std::time::Instant::now();
        let mut handles = Vec::new();

        // 24 readers, hammering get_all_refs + list_locks.
        for _ in 0..24 {
            let db = Arc::clone(&db);
            handles.push(thread::spawn(move || {
                for _ in 0..200 {
                    let refs = db.get_all_refs("alice/pool").unwrap();
                    assert!(refs.len() >= 8);
                    let _ = db.list_locks("alice/pool", "", "").unwrap();
                }
            }));
        }

        // 8 writers, each claiming a disjoint lock path so SQLITE_BUSY
        // should not surface (WAL + BEGIN IMMEDIATE + busy_timeout).
        for i in 0..8 {
            let db = Arc::clone(&db);
            handles.push(thread::spawn(move || {
                for j in 0..25 {
                    let path = format!("Content/path_{i}_{j}.uasset");
                    db.acquire_lock(
                        "alice/pool",
                        &path,
                        &format!("writer-{i}"),
                        "ws",
                        "",
                    )
                    .unwrap()
                    .unwrap_or_else(|_| panic!("lock conflict unexpected in disjoint paths"));
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let elapsed = start.elapsed();
        // Sanity cap: 24 readers × 400 ops + 8 writers × 25 ops = ~9.8K
        // metadata ops. On a workstation SSD this should complete well
        // under two seconds with the pooled design; anything past 10s
        // is a regression to the single-Mutex era.
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "32-way pool workload took {:?}, regression vs pooled design",
            elapsed
        );

        // Writers landed exactly 200 locks.
        let locks = db.list_locks("alice/pool", "", "").unwrap();
        assert_eq!(locks.len(), 200);
    }

    #[test]
    fn stale_sessions_surface_for_sweeping() {
        let (_tmp, db) = fresh_db();
        // Short TTL so the session is immediately stale in wall-clock
        // terms once we query with a future cutoff.
        db.create_upload_session("sid-stale", "alice/forcetest", None, 60).unwrap();
        let now = chrono::Utc::now().timestamp();
        // Pretend 24h have passed.
        let list = db.list_stale_upload_sessions(now + 24 * 3600).unwrap();
        assert!(list.iter().any(|(sid, _)| sid == "sid-stale"));

        // Delete and confirm.
        db.delete_upload_session("sid-stale").unwrap();
        let list = db.list_stale_upload_sessions(now + 24 * 3600).unwrap();
        assert!(list.iter().all(|(sid, _)| sid != "sid-stale"));
    }
}
