// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

/// SQLite database for repos, refs, and locks metadata.
pub struct MetadataDb {
    pub(crate) conn: Mutex<Connection>,
}

impl MetadataDb {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open database at {}", path.display()))?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS repos (
                name TEXT PRIMARY KEY,
                description TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
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

        // Migrate: add assignee column if missing
        let _ = conn.execute("ALTER TABLE issues ADD COLUMN assignee TEXT NOT NULL DEFAULT ''", []);
        let _ = conn.execute("ALTER TABLE pull_requests ADD COLUMN assignee TEXT NOT NULL DEFAULT ''", []);

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.create_actions_tables()?;
        Ok(db)
    }

    // -- Repos --

    pub fn list_repos(&self) -> Result<Vec<RepoRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT name, description, created_at FROM repos")?;
        let rows = stmt.query_map([], |row| {
            Ok(RepoRecord {
                name: row.get(0)?,
                description: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn create_repo(&self, name: &str, description: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        let affected = conn.execute(
            "INSERT OR IGNORE INTO repos (name, description, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![name, description, now],
        )?;
        Ok(affected > 0)
    }

    pub fn get_repo(&self, name: &str) -> Result<Option<RepoRecord>> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .prepare("SELECT name, description, created_at FROM repos WHERE name = ?1")?
            .query_row([name], |row| {
                Ok(RepoRecord {
                    name: row.get(0)?,
                    description: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .ok();
        Ok(result)
    }

    pub fn update_repo(&self, name: &str, new_name: &str, description: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();

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

        conn.execute(
            "UPDATE repos SET name = ?1, description = ?2 WHERE name = ?3",
            rusqlite::params![effective_name, description, name],
        )?;

        // Update refs and locks tables if renamed.
        if !new_name.is_empty() && new_name != name {
            conn.execute(
                "UPDATE refs SET repo = ?1 WHERE repo = ?2",
                rusqlite::params![new_name, name],
            )?;
            conn.execute(
                "UPDATE locks SET repo = ?1 WHERE repo = ?2",
                rusqlite::params![new_name, name],
            )?;
        }

        Ok(true)
    }

    pub fn delete_repo(&self, name: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute("DELETE FROM repos WHERE name = ?1", [name])?;
        conn.execute("DELETE FROM refs WHERE repo = ?1", [name])?;
        conn.execute("DELETE FROM locks WHERE repo = ?1", [name])?;
        Ok(affected > 0)
    }

    // -- Refs --

    pub fn get_ref(&self, repo: &str, name: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT hash FROM refs WHERE repo = ?1 AND name = ?2")?;
        let result = stmt
            .query_row(rusqlite::params![repo, name], |row| row.get::<_, Vec<u8>>(0))
            .ok();
        Ok(result)
    }

    pub fn get_all_refs(&self, repo: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let conn = self.conn.lock().unwrap();
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

    /// Compare-and-swap update. Returns true if the update succeeded.
    pub fn update_ref(&self, repo: &str, name: &str, old_hash: &[u8], new_hash: &[u8]) -> Result<bool> {
        let conn = self.conn.lock().unwrap();

        // Check current value.
        let current: Option<Vec<u8>> = conn
            .prepare("SELECT hash FROM refs WHERE repo = ?1 AND name = ?2")?
            .query_row(rusqlite::params![repo, name], |row| row.get::<_, Vec<u8>>(0))
            .ok();

        let matches = match &current {
            Some(h) => h.as_slice() == old_hash,
            None => old_hash.iter().all(|&b| b == 0), // Zero hash means "expect not to exist"
        };

        if !matches {
            return Ok(false);
        }

        conn.execute(
            "INSERT OR REPLACE INTO refs (repo, name, hash) VALUES (?1, ?2, ?3)",
            rusqlite::params![repo, name, new_hash],
        )?;

        Ok(true)
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
        let conn = self.conn.lock().unwrap();

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
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
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
            "SELECT path, owner, workspace_id, created_at, reason FROM locks WHERE repo = ?1 AND path LIKE ?2 AND owner LIKE ?3"
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
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO issues (repo, title, body, author, status, labels, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6, ?7)",
            rusqlite::params![repo, title, body, author, labels, now, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Get a single issue by ID.
    pub fn get_issue(&self, id: i64) -> Result<Option<IssueRecord>> {
        let conn = self.conn.lock().unwrap();
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

        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO pull_requests (repo, title, body, author, status, source_branch, target_branch, labels, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![repo, title, body, author, source_branch, target_branch, labels, now, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Get a single pull request by ID.
    pub fn get_pull_request(&self, id: i64) -> Result<Option<PullRequestRecord>> {
        let conn = self.conn.lock().unwrap();
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

        let conn = self.conn.lock().unwrap();
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
}
