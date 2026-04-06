// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

/// SQLite database for repos, refs, and locks metadata.
pub struct MetadataDb {
    conn: Mutex<Connection>,
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

        Ok(Self {
            conn: Mutex::new(conn),
        })
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
