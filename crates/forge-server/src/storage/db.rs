// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

/// SQLite database for refs and locks metadata.
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
            CREATE TABLE IF NOT EXISTS refs (
                name TEXT PRIMARY KEY,
                hash BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS locks (
                path TEXT PRIMARY KEY,
                owner TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                reason TEXT
            );
            ",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // -- Refs --

    pub fn get_ref(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT hash FROM refs WHERE name = ?1")?;
        let result = stmt
            .query_row([name], |row| row.get::<_, Vec<u8>>(0))
            .ok();
        Ok(result)
    }

    pub fn get_all_refs(&self) -> Result<Vec<(String, Vec<u8>)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT name, hash FROM refs")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Compare-and-swap update. Returns true if the update succeeded.
    pub fn update_ref(&self, name: &str, old_hash: &[u8], new_hash: &[u8]) -> Result<bool> {
        let conn = self.conn.lock().unwrap();

        // Check current value.
        let current: Option<Vec<u8>> = conn
            .prepare("SELECT hash FROM refs WHERE name = ?1")?
            .query_row([name], |row| row.get::<_, Vec<u8>>(0))
            .ok();

        let matches = match &current {
            Some(h) => h.as_slice() == old_hash,
            None => old_hash.iter().all(|&b| b == 0), // Zero hash means "expect not to exist"
        };

        if !matches {
            return Ok(false);
        }

        conn.execute(
            "INSERT OR REPLACE INTO refs (name, hash) VALUES (?1, ?2)",
            rusqlite::params![name, new_hash],
        )?;

        Ok(true)
    }

    // -- Locks --

    /// Try to acquire a lock. Returns Ok(true) if acquired, Ok(false) with existing lock info if denied.
    pub fn acquire_lock(
        &self,
        path: &str,
        owner: &str,
        workspace_id: &str,
        reason: &str,
    ) -> Result<std::result::Result<(), LockInfo>> {
        let conn = self.conn.lock().unwrap();

        // Check if already locked.
        if let Ok(lock) = conn.prepare("SELECT owner, workspace_id, created_at, reason FROM locks WHERE path = ?1")?
            .query_row([path], |row| {
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
            "INSERT INTO locks (path, owner, workspace_id, created_at, reason) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![path, owner, workspace_id, now, reason],
        )?;

        Ok(Ok(()))
    }

    pub fn release_lock(&self, path: &str, owner: &str, force: bool) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let affected = if force {
            conn.execute("DELETE FROM locks WHERE path = ?1", [path])?
        } else {
            conn.execute(
                "DELETE FROM locks WHERE path = ?1 AND owner = ?2",
                rusqlite::params![path, owner],
            )?
        };
        Ok(affected > 0)
    }

    pub fn list_locks(&self, path_prefix: &str, owner_filter: &str) -> Result<Vec<LockInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut locks = Vec::new();

        // Use a single query with LIKE and optional owner filter.
        // "%" matches everything when prefix is empty.
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
            "SELECT path, owner, workspace_id, created_at, reason FROM locks WHERE path LIKE ?1 AND owner LIKE ?2"
        )?;

        let rows = stmt.query_map(rusqlite::params![prefix_pattern, owner_pattern], |row| {
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
