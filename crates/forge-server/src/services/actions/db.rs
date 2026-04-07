// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! SQLite queries for workflows, runs, steps, artifacts, and releases.

use anyhow::Result;
use crate::storage::db::MetadataDb;

// ── Record types ──

#[derive(Debug, Clone)]
pub struct WorkflowRecord {
    pub id: i64,
    pub repo: String,
    pub name: String,
    pub yaml: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct RunRecord {
    pub id: i64,
    pub repo: String,
    pub workflow_id: i64,
    pub workflow_name: String,
    pub trigger: String,
    pub trigger_ref: String,
    pub commit_hash: String,
    pub status: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub triggered_by: String,
}

#[derive(Debug, Clone)]
pub struct StepRecord {
    pub id: i64,
    pub run_id: i64,
    pub job_name: String,
    pub step_index: i32,
    pub name: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub log: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ArtifactRecord {
    pub id: i64,
    pub run_id: i64,
    pub name: String,
    pub path: String,
    pub size_bytes: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct ReleaseRecord {
    pub id: i64,
    pub repo: String,
    pub run_id: Option<i64>,
    pub tag: String,
    pub name: String,
    pub created_at: i64,
}

// ── Actions DB methods ──

impl MetadataDb {
    /// Create the actions tables. Called once during init.
    pub fn create_actions_tables(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS workflows (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo TEXT NOT NULL,
                name TEXT NOT NULL,
                yaml TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                UNIQUE(repo, name)
            );
            CREATE TABLE IF NOT EXISTS workflow_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo TEXT NOT NULL,
                workflow_id INTEGER NOT NULL,
                trigger TEXT NOT NULL,
                trigger_ref TEXT NOT NULL DEFAULT '',
                commit_hash TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'queued',
                started_at INTEGER,
                finished_at INTEGER,
                created_at INTEGER NOT NULL,
                triggered_by TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS workflow_steps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id INTEGER NOT NULL,
                job_name TEXT NOT NULL,
                step_index INTEGER NOT NULL,
                name TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                exit_code INTEGER,
                log TEXT NOT NULL DEFAULT '',
                started_at INTEGER,
                finished_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS artifacts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                path TEXT NOT NULL,
                size_bytes INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                UNIQUE(run_id, name)
            );
            CREATE TABLE IF NOT EXISTS releases (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo TEXT NOT NULL,
                run_id INTEGER,
                tag TEXT NOT NULL,
                name TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                UNIQUE(repo, tag)
            );
            CREATE TABLE IF NOT EXISTS release_artifacts (
                release_id INTEGER NOT NULL,
                artifact_id INTEGER NOT NULL,
                PRIMARY KEY (release_id, artifact_id)
            );
            ",
        )?;
        Ok(())
    }

    // ── Workflows ──

    pub fn create_workflow(&self, repo: &str, name: &str, yaml: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO workflows (repo, name, yaml, enabled, created_at, updated_at) VALUES (?1, ?2, ?3, 1, ?4, ?4)",
            rusqlite::params![repo, name, yaml, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_workflow(&self, id: i64, name: &str, yaml: &str, enabled: bool) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        let affected = conn.execute(
            "UPDATE workflows SET name = ?1, yaml = ?2, enabled = ?3, updated_at = ?4 WHERE id = ?5",
            rusqlite::params![name, yaml, enabled as i32, now, id],
        )?;
        Ok(affected > 0)
    }

    pub fn delete_workflow(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute("DELETE FROM workflows WHERE id = ?1", [id])?;
        Ok(affected > 0)
    }

    pub fn list_workflows(&self, repo: &str) -> Result<Vec<WorkflowRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repo, name, yaml, enabled, created_at, updated_at FROM workflows WHERE repo = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map([repo], |row| {
            Ok(WorkflowRecord {
                id: row.get(0)?,
                repo: row.get(1)?,
                name: row.get(2)?,
                yaml: row.get(3)?,
                enabled: row.get::<_, i32>(4)? != 0,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_workflow(&self, id: i64) -> Result<Option<WorkflowRecord>> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .prepare("SELECT id, repo, name, yaml, enabled, created_at, updated_at FROM workflows WHERE id = ?1")?
            .query_row([id], |row| {
                Ok(WorkflowRecord {
                    id: row.get(0)?,
                    repo: row.get(1)?,
                    name: row.get(2)?,
                    yaml: row.get(3)?,
                    enabled: row.get::<_, i32>(4)? != 0,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .ok();
        Ok(result)
    }

    pub fn get_enabled_workflows_for_repo(&self, repo: &str) -> Result<Vec<WorkflowRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repo, name, yaml, enabled, created_at, updated_at FROM workflows WHERE repo = ?1 AND enabled = 1",
        )?;
        let rows = stmt.query_map([repo], |row| {
            Ok(WorkflowRecord {
                id: row.get(0)?,
                repo: row.get(1)?,
                name: row.get(2)?,
                yaml: row.get(3)?,
                enabled: row.get::<_, i32>(4)? != 0,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ── Runs ──

    pub fn create_run(
        &self,
        repo: &str,
        workflow_id: i64,
        trigger: &str,
        trigger_ref: &str,
        commit_hash: &str,
        triggered_by: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO workflow_runs (repo, workflow_id, trigger, trigger_ref, commit_hash, status, created_at, triggered_by) VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?7)",
            rusqlite::params![repo, workflow_id, trigger, trigger_ref, commit_hash, now, triggered_by],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_run_status(&self, run_id: i64, status: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        match status {
            "running" => {
                conn.execute(
                    "UPDATE workflow_runs SET status = ?1, started_at = ?2 WHERE id = ?3",
                    rusqlite::params![status, now, run_id],
                )?;
            }
            "success" | "failure" | "cancelled" => {
                conn.execute(
                    "UPDATE workflow_runs SET status = ?1, finished_at = ?2 WHERE id = ?3",
                    rusqlite::params![status, now, run_id],
                )?;
            }
            _ => {
                conn.execute(
                    "UPDATE workflow_runs SET status = ?1 WHERE id = ?2",
                    rusqlite::params![status, run_id],
                )?;
            }
        }
        Ok(())
    }

    pub fn list_runs(&self, repo: &str, workflow_id: i64, limit: i32, offset: i32) -> Result<(Vec<RunRecord>, i32)> {
        let conn = self.conn.lock().unwrap();

        let (where_clause, total) = if workflow_id > 0 {
            let count: i32 = conn
                .prepare("SELECT COUNT(*) FROM workflow_runs WHERE repo = ?1 AND workflow_id = ?2")?
                .query_row(rusqlite::params![repo, workflow_id], |row| row.get(0))?;
            ("WHERE r.repo = ?1 AND r.workflow_id = ?2".to_string(), count)
        } else {
            let count: i32 = conn
                .prepare("SELECT COUNT(*) FROM workflow_runs WHERE repo = ?1")?
                .query_row([repo], |row| row.get(0))?;
            ("WHERE r.repo = ?1".to_string(), count)
        };

        let sql = format!(
            "SELECT r.id, r.repo, r.workflow_id, COALESCE(w.name, ''), r.trigger, r.trigger_ref, r.commit_hash, r.status, r.started_at, r.finished_at, r.created_at, r.triggered_by \
             FROM workflow_runs r LEFT JOIN workflows w ON r.workflow_id = w.id \
             {} ORDER BY r.created_at DESC LIMIT ?3 OFFSET ?4",
            where_clause
        );

        let limit = if limit <= 0 { 50 } else { limit };
        let mut stmt = conn.prepare(&sql)?;
        let rows = if workflow_id > 0 {
            stmt.query_map(rusqlite::params![repo, workflow_id, limit, offset], Self::map_run)?
        } else {
            stmt.query_map(rusqlite::params![repo, 0, limit, offset], Self::map_run)?
        };

        let runs = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok((runs, total))
    }

    fn map_run(row: &rusqlite::Row) -> rusqlite::Result<RunRecord> {
        Ok(RunRecord {
            id: row.get(0)?,
            repo: row.get(1)?,
            workflow_id: row.get(2)?,
            workflow_name: row.get(3)?,
            trigger: row.get(4)?,
            trigger_ref: row.get(5)?,
            commit_hash: row.get(6)?,
            status: row.get(7)?,
            started_at: row.get(8)?,
            finished_at: row.get(9)?,
            created_at: row.get(10)?,
            triggered_by: row.get(11)?,
        })
    }

    pub fn get_run(&self, run_id: i64) -> Result<Option<RunRecord>> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .prepare(
                "SELECT r.id, r.repo, r.workflow_id, COALESCE(w.name, ''), r.trigger, r.trigger_ref, r.commit_hash, r.status, r.started_at, r.finished_at, r.created_at, r.triggered_by \
                 FROM workflow_runs r LEFT JOIN workflows w ON r.workflow_id = w.id WHERE r.id = ?1",
            )?
            .query_row([run_id], Self::map_run)
            .ok();
        Ok(result)
    }

    // ── Steps ──

    pub fn create_step(
        &self,
        run_id: i64,
        job_name: &str,
        step_index: i32,
        name: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO workflow_steps (run_id, job_name, step_index, name, status) VALUES (?1, ?2, ?3, ?4, 'pending')",
            rusqlite::params![run_id, job_name, step_index, name],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_step(&self, step_id: i64, status: &str, exit_code: Option<i32>, log: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        match status {
            "running" => {
                conn.execute(
                    "UPDATE workflow_steps SET status = ?1, started_at = ?2 WHERE id = ?3",
                    rusqlite::params![status, now, step_id],
                )?;
            }
            _ => {
                conn.execute(
                    "UPDATE workflow_steps SET status = ?1, exit_code = ?2, log = ?3, finished_at = ?4 WHERE id = ?5",
                    rusqlite::params![status, exit_code, log, now, step_id],
                )?;
            }
        }
        Ok(())
    }

    pub fn list_steps(&self, run_id: i64) -> Result<Vec<StepRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, run_id, job_name, step_index, name, status, exit_code, log, started_at, finished_at FROM workflow_steps WHERE run_id = ?1 ORDER BY step_index",
        )?;
        let rows = stmt.query_map([run_id], |row| {
            Ok(StepRecord {
                id: row.get(0)?,
                run_id: row.get(1)?,
                job_name: row.get(2)?,
                step_index: row.get(3)?,
                name: row.get(4)?,
                status: row.get(5)?,
                exit_code: row.get(6)?,
                log: row.get(7)?,
                started_at: row.get(8)?,
                finished_at: row.get(9)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ── Artifacts ──

    pub fn create_artifact(&self, run_id: i64, name: &str, path: &str, size_bytes: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO artifacts (run_id, name, path, size_bytes, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![run_id, name, path, size_bytes, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_artifacts(&self, run_id: i64) -> Result<Vec<ArtifactRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, run_id, name, path, size_bytes, created_at FROM artifacts WHERE run_id = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map([run_id], |row| {
            Ok(ArtifactRecord {
                id: row.get(0)?,
                run_id: row.get(1)?,
                name: row.get(2)?,
                path: row.get(3)?,
                size_bytes: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_artifact(&self, artifact_id: i64) -> Result<Option<ArtifactRecord>> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .prepare("SELECT id, run_id, name, path, size_bytes, created_at FROM artifacts WHERE id = ?1")?
            .query_row([artifact_id], |row| {
                Ok(ArtifactRecord {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    name: row.get(2)?,
                    path: row.get(3)?,
                    size_bytes: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .ok();
        Ok(result)
    }

    // ── Releases ──

    pub fn create_release(&self, repo: &str, run_id: Option<i64>, tag: &str, name: &str, artifact_ids: &[i64]) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO releases (repo, run_id, tag, name, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![repo, run_id, tag, name, now],
        )?;
        let release_id = conn.last_insert_rowid();

        for &artifact_id in artifact_ids {
            conn.execute(
                "INSERT OR IGNORE INTO release_artifacts (release_id, artifact_id) VALUES (?1, ?2)",
                rusqlite::params![release_id, artifact_id],
            )?;
        }
        Ok(release_id)
    }

    pub fn list_releases(&self, repo: &str) -> Result<Vec<ReleaseRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repo, run_id, tag, name, created_at FROM releases WHERE repo = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([repo], |row| {
            Ok(ReleaseRecord {
                id: row.get(0)?,
                repo: row.get(1)?,
                run_id: row.get(2)?,
                tag: row.get(3)?,
                name: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_release(&self, release_id: i64) -> Result<Option<ReleaseRecord>> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .prepare("SELECT id, repo, run_id, tag, name, created_at FROM releases WHERE id = ?1")?
            .query_row([release_id], |row| {
                Ok(ReleaseRecord {
                    id: row.get(0)?,
                    repo: row.get(1)?,
                    run_id: row.get(2)?,
                    tag: row.get(3)?,
                    name: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .ok();
        Ok(result)
    }

    pub fn get_release_artifact_ids(&self, release_id: i64) -> Result<Vec<i64>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT artifact_id FROM release_artifacts WHERE release_id = ?1",
        )?;
        let rows = stmt.query_map([release_id], |row| row.get(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }
}
