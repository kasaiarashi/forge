// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under BSL 1.1.

//! Phase 7g full-coverage Postgres impls — issues / pull_requests /
//! comments / workflows / runs / steps / artifacts / releases /
//! agents / secrets / default_branch.
//!
//! These live as **inherent** methods on
//! [`crate::storage::postgres::PgMetadataBackend`] (NOT trait
//! methods), invoked via the [`crate::dispatch_pg_inherent!`] macro
//! from `MetadataDb`'s SQLite-side method bodies. Keeping them off
//! the trait avoids growing `MetadataBackend` past 100 methods —
//! it stays focused on the Phase-1 atomic-push surface where
//! cross-backend parity tests matter.
//!
//! Every method runs on the caller's thread (which `block_pg`
//! guarantees is a fresh OS thread) and uses the shared r2d2 pool.

#![cfg(feature = "postgres")]

use anyhow::{Context, Result};

use super::db::{CommentRecord, IssueRecord, PullRequestRecord};
use super::postgres::PgMetadataBackend;
use crate::services::actions::db::{
    ArtifactRecord, ReleaseRecord, RunRecord, StepRecord, WorkflowRecord,
};

impl PgMetadataBackend {
    fn ic(
        &self,
    ) -> Result<r2d2::PooledConnection<r2d2_postgres::PostgresConnectionManager<postgres::NoTls>>>
    {
        self.pool().get().context("postgres pool get (full)")
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn map_issue_row(row: &postgres::Row) -> IssueRecord {
    IssueRecord {
        id: row.get("id"),
        repo: row.get("repo"),
        title: row.get("title"),
        body: row.get("body"),
        author: row.get("author"),
        status: row.get("status"),
        labels: row.get("labels"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        comment_count: row.get::<_, i32>("comment_count"),
        assignee: row.get::<_, Option<String>>("assignee").unwrap_or_default(),
    }
}

fn map_pr_row(row: &postgres::Row) -> PullRequestRecord {
    PullRequestRecord {
        id: row.get("id"),
        repo: row.get("repo"),
        title: row.get("title"),
        body: row.get("body"),
        author: row.get("author"),
        status: row.get("status"),
        source_branch: row.get("source_branch"),
        target_branch: row.get("target_branch"),
        labels: row.get("labels"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        comment_count: row.get::<_, i32>("comment_count"),
        assignee: row.get::<_, Option<String>>("assignee").unwrap_or_default(),
    }
}

fn map_comment_row(row: &postgres::Row) -> CommentRecord {
    CommentRecord {
        id: row.get("id"),
        repo: row.get("repo"),
        issue_id: row.get("issue_id"),
        kind: row.get("kind"),
        author: row.get("author"),
        body: row.get("body"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

const ISSUE_COLS: &str =
    "id, repo, title, body, author, status, labels, created_at, updated_at, comment_count, assignee";
const PR_COLS: &str =
    "id, repo, title, body, author, status, source_branch, target_branch, labels, created_at, updated_at, comment_count, assignee";
const COMMENT_COLS: &str = "id, repo, issue_id, kind, author, body, created_at, updated_at";

// ── Issues ─────────────────────────────────────────────────────────

impl PgMetadataBackend {
    pub fn list_issues(
        &self,
        repo: &str,
        status: &str,
        limit: i32,
        offset: i32,
    ) -> Result<(Vec<IssueRecord>, i32, i32, i32)> {
        let mut conn = self.ic()?;
        let lim: i64 = if limit <= 0 { 50 } else { limit as i64 };
        let off: i64 = offset as i64;

        let open_count: i64 = conn
            .query_one(
                "SELECT COUNT(*) FROM issues WHERE repo = $1 AND status = 'open'",
                &[&repo],
            )?
            .get(0);
        let closed_count: i64 = conn
            .query_one(
                "SELECT COUNT(*) FROM issues WHERE repo = $1 AND status = 'closed'",
                &[&repo],
            )?
            .get(0);

        let (rows, total) = if status.is_empty() {
            let sql = format!(
                "SELECT {ISSUE_COLS} FROM issues WHERE repo = $1
                 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
            );
            let rs = conn.query(&sql, &[&repo, &lim, &off])?;
            (rs, open_count + closed_count)
        } else {
            let sql = format!(
                "SELECT {ISSUE_COLS} FROM issues WHERE repo = $1 AND status = $4
                 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
            );
            let rs = conn.query(&sql, &[&repo, &lim, &off, &status])?;
            let t = if status == "open" { open_count } else { closed_count };
            (rs, t)
        };

        Ok((
            rows.iter().map(map_issue_row).collect(),
            total as i32,
            open_count as i32,
            closed_count as i32,
        ))
    }

    pub fn get_issue(&self, id: i64) -> Result<Option<IssueRecord>> {
        let mut conn = self.ic()?;
        let sql = format!("SELECT {ISSUE_COLS} FROM issues WHERE id = $1");
        let row = conn.query_opt(&sql, &[&id])?;
        Ok(row.as_ref().map(map_issue_row))
    }

    pub fn create_issue(
        &self,
        repo: &str,
        title: &str,
        body: &str,
        author: &str,
        labels: &str,
    ) -> Result<i64> {
        let mut conn = self.ic()?;
        let now_ts = now();
        let row = conn.query_one(
            "INSERT INTO issues (repo, title, body, author, status, labels, created_at, updated_at)
             VALUES ($1, $2, $3, $4, 'open', $5, $6, $6)
             RETURNING id",
            &[&repo, &title, &body, &author, &labels, &now_ts],
        )?;
        Ok(row.get(0))
    }

    pub fn update_issue(
        &self,
        id: i64,
        title: &str,
        body: &str,
        status: &str,
        labels: &str,
        assignee: &str,
    ) -> Result<bool> {
        let current = self.get_issue(id)?;
        let current = match current {
            Some(c) => c,
            None => return Ok(false),
        };
        let mut conn = self.ic()?;
        let now_ts = now();
        let new_title = if title.is_empty() { &current.title } else { title };
        let new_body = if body.is_empty() { &current.body } else { body };
        let new_status = if status.is_empty() { &current.status } else { status };
        let new_labels = if labels.is_empty() { &current.labels } else { labels };
        let new_assignee = if assignee.is_empty() { &current.assignee } else { assignee };
        let n = conn.execute(
            "UPDATE issues
             SET title = $1, body = $2, status = $3, labels = $4, assignee = $5, updated_at = $6
             WHERE id = $7",
            &[
                &new_title,
                &new_body,
                &new_status,
                &new_labels,
                &new_assignee,
                &now_ts,
                &id,
            ],
        )?;
        Ok(n > 0)
    }
}

// ── Pull requests ──────────────────────────────────────────────────

impl PgMetadataBackend {
    pub fn list_pull_requests(
        &self,
        repo: &str,
        status: &str,
        limit: i32,
        offset: i32,
    ) -> Result<(Vec<PullRequestRecord>, i32, i32, i32)> {
        let mut conn = self.ic()?;
        let lim: i64 = if limit <= 0 { 50 } else { limit as i64 };
        let off: i64 = offset as i64;

        let open_count: i64 = conn
            .query_one(
                "SELECT COUNT(*) FROM pull_requests WHERE repo = $1 AND status = 'open'",
                &[&repo],
            )?
            .get(0);
        let closed_count: i64 = conn
            .query_one(
                "SELECT COUNT(*) FROM pull_requests
                 WHERE repo = $1 AND (status = 'closed' OR status = 'merged')",
                &[&repo],
            )?
            .get(0);

        let (rows, total) = if status.is_empty() {
            let sql = format!(
                "SELECT {PR_COLS} FROM pull_requests WHERE repo = $1
                 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
            );
            let rs = conn.query(&sql, &[&repo, &lim, &off])?;
            (rs, open_count + closed_count)
        } else {
            let sql = format!(
                "SELECT {PR_COLS} FROM pull_requests WHERE repo = $1 AND status = $4
                 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
            );
            let rs = conn.query(&sql, &[&repo, &lim, &off, &status])?;
            let t = if status == "open" { open_count } else { closed_count };
            (rs, t)
        };

        Ok((
            rows.iter().map(map_pr_row).collect(),
            total as i32,
            open_count as i32,
            closed_count as i32,
        ))
    }

    pub fn get_pull_request(&self, id: i64) -> Result<Option<PullRequestRecord>> {
        let mut conn = self.ic()?;
        let sql = format!("SELECT {PR_COLS} FROM pull_requests WHERE id = $1");
        let row = conn.query_opt(&sql, &[&id])?;
        Ok(row.as_ref().map(map_pr_row))
    }

    pub fn create_pull_request(
        &self,
        repo: &str,
        title: &str,
        body: &str,
        author: &str,
        source_branch: &str,
        target_branch: &str,
        labels: &str,
    ) -> Result<i64> {
        let mut conn = self.ic()?;
        let now_ts = now();
        let row = conn.query_one(
            "INSERT INTO pull_requests
                (repo, title, body, author, status, source_branch, target_branch,
                 labels, created_at, updated_at)
             VALUES ($1, $2, $3, $4, 'open', $5, $6, $7, $8, $8)
             RETURNING id",
            &[
                &repo,
                &title,
                &body,
                &author,
                &source_branch,
                &target_branch,
                &labels,
                &now_ts,
            ],
        )?;
        Ok(row.get(0))
    }

    pub fn update_pull_request(
        &self,
        id: i64,
        title: &str,
        body: &str,
        status: &str,
        labels: &str,
        assignee: &str,
    ) -> Result<bool> {
        let current = self.get_pull_request(id)?;
        let current = match current {
            Some(c) => c,
            None => return Ok(false),
        };
        let mut conn = self.ic()?;
        let now_ts = now();
        let new_title = if title.is_empty() { &current.title } else { title };
        let new_body = if body.is_empty() { &current.body } else { body };
        let new_status = if status.is_empty() { &current.status } else { status };
        let new_labels = if labels.is_empty() { &current.labels } else { labels };
        let new_assignee = if assignee.is_empty() { &current.assignee } else { assignee };
        let n = conn.execute(
            "UPDATE pull_requests
             SET title = $1, body = $2, status = $3, labels = $4, assignee = $5, updated_at = $6
             WHERE id = $7",
            &[
                &new_title,
                &new_body,
                &new_status,
                &new_labels,
                &new_assignee,
                &now_ts,
                &id,
            ],
        )?;
        Ok(n > 0)
    }
}

// ── Comments ───────────────────────────────────────────────────────

impl PgMetadataBackend {
    pub fn list_comments(
        &self,
        repo: &str,
        issue_id: i64,
        kind: &str,
    ) -> Result<Vec<CommentRecord>> {
        let mut conn = self.ic()?;
        let sql = format!(
            "SELECT {COMMENT_COLS} FROM comments
             WHERE repo = $1 AND issue_id = $2 AND kind = $3
             ORDER BY created_at ASC"
        );
        let rows = conn.query(&sql, &[&repo, &issue_id, &kind])?;
        Ok(rows.iter().map(map_comment_row).collect())
    }

    pub fn get_comment(&self, id: i64) -> Result<Option<CommentRecord>> {
        let mut conn = self.ic()?;
        let sql = format!("SELECT {COMMENT_COLS} FROM comments WHERE id = $1");
        let row = conn.query_opt(&sql, &[&id])?;
        Ok(row.as_ref().map(map_comment_row))
    }

    pub fn create_comment(
        &self,
        repo: &str,
        issue_id: i64,
        kind: &str,
        author: &str,
        body: &str,
    ) -> Result<i64> {
        let mut conn = self.ic()?;
        let now_ts = now();
        let row = conn.query_one(
            "INSERT INTO comments (repo, issue_id, kind, author, body, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $6) RETURNING id",
            &[&repo, &issue_id, &kind, &author, &body, &now_ts],
        )?;
        let id: i64 = row.get(0);
        let table = if kind == "pull_request" {
            "pull_requests"
        } else {
            "issues"
        };
        conn.execute(
            &format!("UPDATE {table} SET comment_count = comment_count + 1 WHERE id = $1"),
            &[&issue_id],
        )?;
        Ok(id)
    }

    pub fn update_comment(&self, id: i64, body: &str) -> Result<bool> {
        let mut conn = self.ic()?;
        let now_ts = now();
        let n = conn.execute(
            "UPDATE comments SET body = $1, updated_at = $2 WHERE id = $3",
            &[&body, &now_ts, &id],
        )?;
        Ok(n > 0)
    }

    pub fn delete_comment(&self, id: i64) -> Result<bool> {
        let mut conn = self.ic()?;
        let parent: Option<(i64, String)> = conn
            .query_opt(
                "SELECT issue_id, kind FROM comments WHERE id = $1",
                &[&id],
            )?
            .map(|r| (r.get("issue_id"), r.get("kind")));
        let n = conn.execute("DELETE FROM comments WHERE id = $1", &[&id])?;
        if n > 0 {
            if let Some((issue_id, kind)) = parent {
                let table = if kind == "pull_request" {
                    "pull_requests"
                } else {
                    "issues"
                };
                conn.execute(
                    &format!(
                        "UPDATE {table} SET comment_count =
                         CASE WHEN comment_count > 0 THEN comment_count - 1 ELSE 0 END
                         WHERE id = $1"
                    ),
                    &[&issue_id],
                )?;
            }
        }
        Ok(n > 0)
    }
}

// ── Default branch ─────────────────────────────────────────────────

impl PgMetadataBackend {
    pub fn get_default_branch(&self, repo: &str) -> Result<String> {
        let mut conn = self.ic()?;
        let row = conn.query_opt("SELECT default_branch FROM repos WHERE name = $1", &[&repo])?;
        Ok(row
            .map(|r| r.get::<_, Option<String>>(0).unwrap_or_default())
            .unwrap_or_default())
    }

    pub fn set_default_branch(&self, repo: &str, branch: &str) -> Result<bool> {
        let mut conn = self.ic()?;
        let n = conn.execute(
            "UPDATE repos SET default_branch = $1 WHERE name = $2",
            &[&branch, &repo],
        )?;
        Ok(n > 0)
    }
}

// ── Workflows ──────────────────────────────────────────────────────

fn map_workflow_row(row: &postgres::Row) -> WorkflowRecord {
    WorkflowRecord {
        id: row.get("id"),
        repo: row.get("repo"),
        name: row.get("name"),
        yaml: row.get("yaml"),
        enabled: row.get::<_, i32>("enabled") != 0,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

const WORKFLOW_COLS: &str = "id, repo, name, yaml, enabled, created_at, updated_at";

impl PgMetadataBackend {
    pub fn create_workflow(&self, repo: &str, name: &str, yaml: &str) -> Result<i64> {
        let mut conn = self.ic()?;
        let now_ts = now();
        let row = conn.query_one(
            "INSERT INTO workflows (repo, name, yaml, enabled, created_at, updated_at)
             VALUES ($1, $2, $3, 1, $4, $4) RETURNING id",
            &[&repo, &name, &yaml, &now_ts],
        )?;
        Ok(row.get(0))
    }

    pub fn update_workflow(
        &self,
        id: i64,
        name: &str,
        yaml: &str,
        enabled: bool,
    ) -> Result<bool> {
        let mut conn = self.ic()?;
        let now_ts = now();
        let enabled_i: i32 = if enabled { 1 } else { 0 };
        let n = conn.execute(
            "UPDATE workflows SET name = $1, yaml = $2, enabled = $3, updated_at = $4
             WHERE id = $5",
            &[&name, &yaml, &enabled_i, &now_ts, &id],
        )?;
        Ok(n > 0)
    }

    pub fn delete_workflow(&self, id: i64) -> Result<bool> {
        let mut conn = self.ic()?;
        let n = conn.execute("DELETE FROM workflows WHERE id = $1", &[&id])?;
        Ok(n > 0)
    }

    pub fn list_workflows(&self, repo: &str) -> Result<Vec<WorkflowRecord>> {
        let mut conn = self.ic()?;
        let sql = format!("SELECT {WORKFLOW_COLS} FROM workflows WHERE repo = $1 ORDER BY name");
        let rows = conn.query(&sql, &[&repo])?;
        Ok(rows.iter().map(map_workflow_row).collect())
    }

    pub fn get_workflow(&self, id: i64) -> Result<Option<WorkflowRecord>> {
        let mut conn = self.ic()?;
        let sql = format!("SELECT {WORKFLOW_COLS} FROM workflows WHERE id = $1");
        let row = conn.query_opt(&sql, &[&id])?;
        Ok(row.as_ref().map(map_workflow_row))
    }

    pub fn get_enabled_workflows_for_repo(&self, repo: &str) -> Result<Vec<WorkflowRecord>> {
        let mut conn = self.ic()?;
        let sql = format!(
            "SELECT {WORKFLOW_COLS} FROM workflows WHERE repo = $1 AND enabled = 1"
        );
        let rows = conn.query(&sql, &[&repo])?;
        Ok(rows.iter().map(map_workflow_row).collect())
    }
}

// ── Workflow runs ──────────────────────────────────────────────────

fn map_run_row(row: &postgres::Row, workflow_name: String) -> RunRecord {
    RunRecord {
        id: row.get("id"),
        repo: row.get("repo"),
        workflow_id: row.get("workflow_id"),
        workflow_name,
        trigger: row.get("trigger"),
        trigger_ref: row.get("trigger_ref"),
        commit_hash: row.get("commit_hash"),
        status: row.get("status"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        created_at: row.get("created_at"),
        triggered_by: row.get("triggered_by"),
    }
}

impl PgMetadataBackend {
    pub fn create_run(
        &self,
        repo: &str,
        workflow_id: i64,
        trigger: &str,
        trigger_ref: &str,
        commit_hash: &str,
        triggered_by: &str,
    ) -> Result<i64> {
        let mut conn = self.ic()?;
        let now_ts = now();
        let row = conn.query_one(
            "INSERT INTO workflow_runs
                (repo, workflow_id, trigger, trigger_ref, commit_hash, status, created_at, triggered_by)
             VALUES ($1, $2, $3, $4, $5, 'queued', $6, $7) RETURNING id",
            &[
                &repo,
                &workflow_id,
                &trigger,
                &trigger_ref,
                &commit_hash,
                &now_ts,
                &triggered_by,
            ],
        )?;
        Ok(row.get(0))
    }

    pub fn update_run_status(&self, id: i64, status: &str) -> Result<()> {
        let mut conn = self.ic()?;
        let now_ts = now();
        match status {
            "running" => {
                conn.execute(
                    "UPDATE workflow_runs SET status = $1, started_at = $2 WHERE id = $3",
                    &[&status, &now_ts, &id],
                )?;
            }
            "success" | "failed" | "cancelled" => {
                conn.execute(
                    "UPDATE workflow_runs SET status = $1, finished_at = $2 WHERE id = $3",
                    &[&status, &now_ts, &id],
                )?;
            }
            _ => {
                conn.execute(
                    "UPDATE workflow_runs SET status = $1 WHERE id = $2",
                    &[&status, &id],
                )?;
            }
        }
        Ok(())
    }

    pub fn get_run(&self, id: i64) -> Result<Option<RunRecord>> {
        let mut conn = self.ic()?;
        let row = conn.query_opt(
            "SELECT r.id, r.repo, r.workflow_id, r.trigger, r.trigger_ref, r.commit_hash,
                    r.status, r.started_at, r.finished_at, r.created_at, r.triggered_by,
                    COALESCE(w.name, '') AS workflow_name
             FROM workflow_runs r LEFT JOIN workflows w ON w.id = r.workflow_id
             WHERE r.id = $1",
            &[&id],
        )?;
        Ok(row.map(|r| {
            let name: String = r.get("workflow_name");
            map_run_row(&r, name)
        }))
    }

    pub fn list_runs(
        &self,
        repo: &str,
        workflow_id: i64,
        limit: i32,
        offset: i32,
    ) -> Result<(Vec<RunRecord>, i32)> {
        let mut conn = self.ic()?;
        let lim: i64 = if limit <= 0 { 50 } else { limit as i64 };
        let off: i64 = offset as i64;
        let (total, rows) = if workflow_id > 0 {
            let count: i64 = conn
                .query_one(
                    "SELECT COUNT(*) FROM workflow_runs WHERE repo = $1 AND workflow_id = $2",
                    &[&repo, &workflow_id],
                )?
                .get(0);
            let rs = conn.query(
                "SELECT r.id, r.repo, r.workflow_id, r.trigger, r.trigger_ref,
                        r.commit_hash, r.status, r.started_at, r.finished_at,
                        r.created_at, r.triggered_by,
                        COALESCE(w.name, '') AS workflow_name
                 FROM workflow_runs r LEFT JOIN workflows w ON w.id = r.workflow_id
                 WHERE r.repo = $1 AND r.workflow_id = $2
                 ORDER BY r.created_at DESC LIMIT $3 OFFSET $4",
                &[&repo, &workflow_id, &lim, &off],
            )?;
            (count, rs)
        } else {
            let count: i64 = conn
                .query_one(
                    "SELECT COUNT(*) FROM workflow_runs WHERE repo = $1",
                    &[&repo],
                )?
                .get(0);
            let rs = conn.query(
                "SELECT r.id, r.repo, r.workflow_id, r.trigger, r.trigger_ref,
                        r.commit_hash, r.status, r.started_at, r.finished_at,
                        r.created_at, r.triggered_by,
                        COALESCE(w.name, '') AS workflow_name
                 FROM workflow_runs r LEFT JOIN workflows w ON w.id = r.workflow_id
                 WHERE r.repo = $1
                 ORDER BY r.created_at DESC LIMIT $2 OFFSET $3",
                &[&repo, &lim, &off],
            )?;
            (count, rs)
        };
        Ok((
            rows.into_iter()
                .map(|r| {
                    let name: String = r.get("workflow_name");
                    map_run_row(&r, name)
                })
                .collect(),
            total as i32,
        ))
    }

    pub fn requeue_stale_runs(&self, cutoff_ts: i64) -> Result<usize> {
        let mut conn = self.ic()?;
        let mut tx = conn.transaction()?;
        let stale = tx.query(
            "SELECT c.run_id FROM run_claims c
             JOIN agents a ON a.id = c.agent_id
             WHERE COALESCE(a.last_seen, 0) < $1",
            &[&cutoff_ts],
        )?;
        let mut requeued = 0usize;
        for row in stale {
            let run_id: i64 = row.get(0);
            tx.execute("DELETE FROM run_claims WHERE run_id = $1", &[&run_id])?;
            let n = tx.execute(
                "UPDATE workflow_runs
                 SET status = 'queued', started_at = NULL
                 WHERE id = $1 AND status = 'running'",
                &[&run_id],
            )?;
            if n > 0 {
                requeued += 1;
            }
        }
        tx.commit()?;
        Ok(requeued)
    }

    pub fn claim_next_run(
        &self,
        agent_id: i64,
        _agent_labels: &[String],
    ) -> Result<Option<i64>> {
        let mut conn = self.ic()?;
        let mut tx = conn.transaction()?;
        let candidate: Option<i64> = tx
            .query_opt(
                "SELECT r.id FROM workflow_runs r
                 LEFT JOIN run_claims c ON c.run_id = r.id
                 WHERE r.status = 'queued' AND c.run_id IS NULL
                 ORDER BY r.created_at ASC
                 LIMIT 1
                 FOR UPDATE OF r SKIP LOCKED",
                &[],
            )?
            .map(|r| r.get(0));
        if let Some(run_id) = candidate {
            let now_ts = now();
            tx.execute(
                "INSERT INTO run_claims (run_id, agent_id, claimed_at) VALUES ($1, $2, $3)",
                &[&run_id, &agent_id, &now_ts],
            )?;
            tx.execute(
                "UPDATE workflow_runs SET status = 'running', started_at = $1 WHERE id = $2",
                &[&now_ts, &run_id],
            )?;
            tx.commit()?;
            Ok(Some(run_id))
        } else {
            Ok(None)
        }
    }

    pub fn get_run_claim_agent(&self, run_id: i64) -> Result<Option<i64>> {
        let mut conn = self.ic()?;
        let row = conn.query_opt(
            "SELECT agent_id FROM run_claims WHERE run_id = $1",
            &[&run_id],
        )?;
        Ok(row.and_then(|r| r.get::<_, Option<i64>>(0)))
    }
}

// ── Workflow steps ─────────────────────────────────────────────────

fn map_step_row(row: &postgres::Row) -> StepRecord {
    StepRecord {
        id: row.get("id"),
        job_name: row.get("job_name"),
        step_index: row.get("step_index"),
        name: row.get("name"),
        status: row.get("status"),
        exit_code: row.get("exit_code"),
        log: row.get("log"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
    }
}

impl PgMetadataBackend {
    pub fn create_step(
        &self,
        run_id: i64,
        job_name: &str,
        step_index: i32,
        name: &str,
    ) -> Result<i64> {
        let mut conn = self.ic()?;
        let row = conn.query_one(
            "INSERT INTO workflow_steps (run_id, job_name, step_index, name, status)
             VALUES ($1, $2, $3, $4, 'pending') RETURNING id",
            &[&run_id, &job_name, &step_index, &name],
        )?;
        Ok(row.get(0))
    }

    pub fn update_step(
        &self,
        step_id: i64,
        status: &str,
        exit_code: Option<i32>,
        log: &str,
    ) -> Result<()> {
        let mut conn = self.ic()?;
        let now_ts = now();
        match status {
            "running" => {
                conn.execute(
                    "UPDATE workflow_steps SET status = $1, started_at = $2 WHERE id = $3",
                    &[&status, &now_ts, &step_id],
                )?;
            }
            _ => {
                conn.execute(
                    "UPDATE workflow_steps
                     SET status = $1, exit_code = $2, log = $3, finished_at = $4
                     WHERE id = $5",
                    &[&status, &exit_code, &log, &now_ts, &step_id],
                )?;
            }
        }
        Ok(())
    }

    pub fn list_steps(&self, run_id: i64) -> Result<Vec<StepRecord>> {
        let mut conn = self.ic()?;
        let rows = conn.query(
            "SELECT id, job_name, step_index, name, status, exit_code, log,
                    started_at, finished_at
             FROM workflow_steps WHERE run_id = $1 ORDER BY step_index",
            &[&run_id],
        )?;
        Ok(rows.iter().map(map_step_row).collect())
    }
}

// ── Artifacts ──────────────────────────────────────────────────────

fn map_artifact_row(row: &postgres::Row) -> ArtifactRecord {
    ArtifactRecord {
        id: row.get("id"),
        run_id: row.get("run_id"),
        name: row.get("name"),
        size_bytes: row.get("size_bytes"),
        created_at: row.get("created_at"),
    }
}

impl PgMetadataBackend {
    pub fn create_artifact(
        &self,
        run_id: i64,
        name: &str,
        path: &str,
        size_bytes: i64,
    ) -> Result<i64> {
        let mut conn = self.ic()?;
        let now_ts = now();
        let row = conn.query_one(
            "INSERT INTO artifacts (run_id, name, path, size_bytes, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (run_id, name) DO UPDATE
                SET path = EXCLUDED.path,
                    size_bytes = EXCLUDED.size_bytes
             RETURNING id",
            &[&run_id, &name, &path, &size_bytes, &now_ts],
        )?;
        Ok(row.get(0))
    }

    pub fn list_artifacts(&self, run_id: i64) -> Result<Vec<ArtifactRecord>> {
        let mut conn = self.ic()?;
        let rows = conn.query(
            "SELECT id, run_id, name, size_bytes, created_at
             FROM artifacts WHERE run_id = $1 ORDER BY name",
            &[&run_id],
        )?;
        Ok(rows.iter().map(map_artifact_row).collect())
    }

    pub fn get_artifact(&self, id: i64) -> Result<Option<ArtifactRecord>> {
        let mut conn = self.ic()?;
        let row = conn.query_opt(
            "SELECT id, run_id, name, size_bytes, created_at
             FROM artifacts WHERE id = $1",
            &[&id],
        )?;
        Ok(row.as_ref().map(map_artifact_row))
    }

    pub fn delete_run_artifacts(&self, run_id: i64) -> Result<usize> {
        let mut conn = self.ic()?;
        let n = conn.execute("DELETE FROM artifacts WHERE run_id = $1", &[&run_id])?;
        Ok(n as usize)
    }

    pub fn retention_candidates(
        &self,
        cutoff_ts: i64,
        keep_per_workflow: i64,
    ) -> Result<Vec<i64>> {
        let mut conn = self.ic()?;
        let rows = conn.query(
            "WITH ranked AS (
                 SELECT a.id AS artifact_id,
                        a.run_id,
                        a.created_at,
                        r.workflow_id,
                        ROW_NUMBER() OVER (
                            PARTITION BY r.workflow_id
                            ORDER BY r.created_at DESC
                        ) AS rn
                 FROM artifacts a
                 JOIN workflow_runs r ON r.id = a.run_id
             )
             SELECT artifact_id FROM ranked
             WHERE created_at < $1 AND rn > $2",
            &[&cutoff_ts, &keep_per_workflow],
        )?;
        Ok(rows.into_iter().map(|r| r.get::<_, i64>(0)).collect())
    }
}

// ── Releases ───────────────────────────────────────────────────────

fn map_release_row(row: &postgres::Row) -> ReleaseRecord {
    ReleaseRecord {
        id: row.get("id"),
        repo: row.get("repo"),
        run_id: row.get("run_id"),
        tag: row.get("tag"),
        name: row.get("name"),
        created_at: row.get("created_at"),
    }
}

impl PgMetadataBackend {
    pub fn create_release(
        &self,
        repo: &str,
        run_id: Option<i64>,
        tag: &str,
        name: &str,
        artifact_ids: &[i64],
    ) -> Result<i64> {
        let mut conn = self.ic()?;
        let now_ts = now();
        let mut tx = conn.transaction()?;
        let row = tx.query_one(
            "INSERT INTO releases (repo, run_id, tag, name, created_at)
             VALUES ($1, $2, $3, $4, $5) RETURNING id",
            &[&repo, &run_id, &tag, &name, &now_ts],
        )?;
        let release_id: i64 = row.get(0);
        for aid in artifact_ids {
            tx.execute(
                "INSERT INTO release_artifacts (release_id, artifact_id)
                 VALUES ($1, $2) ON CONFLICT DO NOTHING",
                &[&release_id, aid],
            )?;
        }
        tx.commit()?;
        Ok(release_id)
    }

    pub fn list_releases(&self, repo: &str) -> Result<Vec<ReleaseRecord>> {
        let mut conn = self.ic()?;
        let rows = conn.query(
            "SELECT id, repo, run_id, tag, name, created_at
             FROM releases WHERE repo = $1 ORDER BY created_at DESC",
            &[&repo],
        )?;
        Ok(rows.iter().map(map_release_row).collect())
    }

    pub fn get_release(&self, id: i64) -> Result<Option<ReleaseRecord>> {
        let mut conn = self.ic()?;
        let row = conn.query_opt(
            "SELECT id, repo, run_id, tag, name, created_at FROM releases WHERE id = $1",
            &[&id],
        )?;
        Ok(row.as_ref().map(map_release_row))
    }

    pub fn get_release_artifact_ids(&self, release_id: i64) -> Result<Vec<i64>> {
        let mut conn = self.ic()?;
        let rows = conn.query(
            "SELECT artifact_id FROM release_artifacts WHERE release_id = $1",
            &[&release_id],
        )?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }
}

// ── Agents ─────────────────────────────────────────────────────────

impl PgMetadataBackend {
    pub fn upsert_agent(
        &self,
        name: &str,
        token_hash: &str,
        labels_json: &str,
        version: &str,
        os: &str,
    ) -> Result<i64> {
        let mut conn = self.ic()?;
        let now_ts = now();
        let row = conn.query_one(
            "INSERT INTO agents
                (name, token_hash, labels_json, version, os, last_seen, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $6)
             ON CONFLICT (name) DO UPDATE
                SET labels_json = EXCLUDED.labels_json,
                    version = EXCLUDED.version,
                    os = EXCLUDED.os,
                    last_seen = EXCLUDED.last_seen
             RETURNING id",
            &[&name, &token_hash, &labels_json, &version, &os, &now_ts],
        )?;
        Ok(row.get(0))
    }

    pub fn get_agent_by_name(&self, name: &str) -> Result<Option<(i64, String, String)>> {
        let mut conn = self.ic()?;
        let row = conn.query_opt(
            "SELECT id, token_hash, labels_json FROM agents WHERE name = $1",
            &[&name],
        )?;
        Ok(row.map(|r| {
            (
                r.get::<_, i64>(0),
                r.get::<_, String>(1),
                r.get::<_, String>(2),
            )
        }))
    }

    pub fn get_agent_by_id(&self, id: i64) -> Result<Option<(String, String, String)>> {
        let mut conn = self.ic()?;
        let row = conn.query_opt(
            "SELECT name, token_hash, labels_json FROM agents WHERE id = $1",
            &[&id],
        )?;
        Ok(row.map(|r| {
            (
                r.get::<_, String>(0),
                r.get::<_, String>(1),
                r.get::<_, String>(2),
            )
        }))
    }

    pub fn touch_agent_last_seen(&self, id: i64) -> Result<()> {
        let mut conn = self.ic()?;
        conn.execute(
            "UPDATE agents SET last_seen = $1 WHERE id = $2",
            &[&now(), &id],
        )?;
        Ok(())
    }

    pub fn list_agents(&self) -> Result<Vec<(i64, String, String, i64, String, String)>> {
        let mut conn = self.ic()?;
        let rows = conn.query(
            "SELECT id, name, labels_json, COALESCE(last_seen, 0), version, os
             FROM agents ORDER BY name",
            &[],
        )?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.get::<_, i64>(0),
                    r.get::<_, String>(1),
                    r.get::<_, String>(2),
                    r.get::<_, i64>(3),
                    r.get::<_, String>(4),
                    r.get::<_, String>(5),
                )
            })
            .collect())
    }

    pub fn delete_agent(&self, id: i64) -> Result<bool> {
        let mut conn = self.ic()?;
        let n = conn.execute("DELETE FROM agents WHERE id = $1", &[&id])?;
        Ok(n > 0)
    }
}

// ── Secrets (raw cipher rows; encryption happens above this layer) ──

impl PgMetadataBackend {
    pub fn upsert_secret(
        &self,
        repo: &str,
        key: &str,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<()> {
        let mut conn = self.ic()?;
        let now_ts = now();
        conn.execute(
            "INSERT INTO secrets (repo, key, nonce, ciphertext, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $5)
             ON CONFLICT (repo, key) DO UPDATE
                SET nonce = EXCLUDED.nonce,
                    ciphertext = EXCLUDED.ciphertext,
                    updated_at = EXCLUDED.updated_at",
            &[&repo, &key, &nonce, &ciphertext, &now_ts],
        )?;
        Ok(())
    }

    pub fn get_secret(&self, repo: &str, key: &str) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        let mut conn = self.ic()?;
        let row = conn.query_opt(
            "SELECT nonce, ciphertext FROM secrets WHERE repo = $1 AND key = $2",
            &[&repo, &key],
        )?;
        Ok(row.map(|r| (r.get::<_, Vec<u8>>(0), r.get::<_, Vec<u8>>(1))))
    }

    pub fn delete_secret(&self, repo: &str, key: &str) -> Result<bool> {
        let mut conn = self.ic()?;
        let n = conn.execute(
            "DELETE FROM secrets WHERE repo = $1 AND key = $2",
            &[&repo, &key],
        )?;
        Ok(n > 0)
    }

    pub fn list_secret_keys(
        &self,
        repo: &str,
    ) -> Result<Vec<crate::services::secrets::SecretMeta>> {
        let mut conn = self.ic()?;
        let rows = conn.query(
            "SELECT repo, key, created_at, updated_at FROM secrets
             WHERE repo = $1 ORDER BY key",
            &[&repo],
        )?;
        Ok(rows
            .into_iter()
            .map(|r| crate::services::secrets::SecretMeta {
                repo: r.get(0),
                key: r.get(1),
                created_at: r.get(2),
                updated_at: r.get(3),
            })
            .collect())
    }
}
