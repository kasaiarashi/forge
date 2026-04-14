// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// SQLite database for repos, refs, and locks metadata.
pub struct MetadataDb {
    pub(crate) conn: Mutex<Connection>,
}

impl MetadataDb {
    /// Acquire the database connection lock, converting poison errors to anyhow errors.
    pub(crate) fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|e| anyhow::anyhow!("database lock poisoned: {e}"))
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open database at {}", path.display()))?;

        conn.pragma_update(None, "journal_mode", "WAL")
            .with_context(|| "Failed to enable WAL mode")?;

        // Enforce foreign-key constraints. SQLite leaves this off per
        // connection by default; the auth tables (sessions / pats /
        // repo_acls) rely on ON DELETE CASCADE to clean up after a user
        // is deleted, so this must be on.
        conn.pragma_update(None, "foreign_keys", "ON")
            .with_context(|| "Failed to enable foreign_keys pragma")?;

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

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.create_actions_tables()?;
        db.create_secrets_tables()?;
        Ok(db)
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

        let tx = conn.transaction()?;

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
        let tx = conn.transaction()?;
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
}
