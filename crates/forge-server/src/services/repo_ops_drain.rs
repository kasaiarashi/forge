// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Background drain for the Phase 3b.5 pending-repo-ops queue.
//!
//! S3 has no atomic "rename prefix" or "delete prefix" primitive, so
//! `S3RepoStorage::rename_repo` / `delete_repo` push work onto the
//! `pending_repo_ops` table and return fast. This task tails the
//! queue, claims ops one at a time with a visibility-timeout, and
//! walks the bucket keyspace with CopyObject + batched DeleteObjects.
//!
//! **Durability.** Each op is a row in SQLite/Postgres — a kill -9
//! mid-drain leaves the row in place (with `not_before` pointing to
//! `claim_time + visibility_secs`); once the visibility window
//! expires, the next worker claims the op and resumes. No lost work,
//! no stranded keys.
//!
//! **Backoff.** A failed op bumps `attempts` and pushes `not_before`
//! out by `backoff_for_attempts`. Permanent failures (bucket gone,
//! creds revoked) will keep flapping with a ceiling of a few minutes
//! between retries; operators can query `list_pending_repo_ops` to
//! spot rows with high `attempts`.
//!
//! Only spawned when the server runs with `[objects] backend = "s3"`.
//! FS deployments have no use for this — `std::fs::rename` + recursive
//! `remove_dir_all` already cover rename/delete synchronously.

#![cfg(feature = "s3-objects")]

use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

use crate::storage::backend::MetadataBackend;
use crate::storage::s3_objects::S3ObjectBackend;

/// How long a claimed op is hidden from other workers. Picked so a
/// single drain cycle for a realistic repo (a few million objects)
/// can complete well inside it — list_objects_v2 paginates at 1000
/// keys per round-trip, and we do 1 delete_objects per page. 10 min
/// leaves headroom even on a slow link.
const VISIBILITY_SECS: i64 = 10 * 60;

/// Sleep between empty-queue polls. The drain work is not latency-
/// sensitive — a 30 s lag on reclaiming a deleted repo's bucket
/// keys costs nothing vs polling every second.
const IDLE_POLL_SECS: u64 = 30;

/// Exponential backoff cap in seconds. Stops a permanently-broken
/// op (bucket vanished, creds rotated) from hammering S3 every
/// 30 seconds forever.
const BACKOFF_CAP_SECS: i64 = 10 * 60;

/// Convert an `attempts` count into a retry delay in seconds.
/// `attempts` includes the current attempt — the first failure comes
/// in with attempts=1, so start the backoff at 30 s.
fn backoff_for_attempts(attempts: i32) -> i64 {
    // 30, 60, 120, 240, 480, 600, 600, ... (capped).
    // Clamp to >=1 before computing the exponent so a negative or
    // zero `attempts` (shouldn't happen but easy to defend against)
    // yields the minimum delay, not an i64 wraparound.
    let clamped = attempts.max(1);
    let exponent = (clamped - 1).min(8) as u32;
    let secs = 30i64.saturating_mul(1i64 << exponent);
    secs.min(BACKOFF_CAP_SECS)
}

/// Spawn the drain loop on the current tokio runtime. Safe to call
/// once from `serve_inner` when the object backend is S3; the task
/// lives for the lifetime of the process.
pub fn spawn(db: Arc<dyn MetadataBackend>, s3: Arc<S3ObjectBackend>) {
    tokio::spawn(async move {
        info!("repo-ops drain started (S3 rename/delete queue)");
        loop {
            match drain_once(&*db, &*s3).await {
                Ok(0) => {
                    // Nothing to do — back off before polling again.
                    sleep(Duration::from_secs(IDLE_POLL_SECS)).await;
                }
                Ok(_) => {
                    // Processed at least one op; immediately try the
                    // next to work through a backlog without artificial
                    // delay.
                }
                Err(e) => {
                    // drain_once itself errored — DB unreachable or
                    // similar. Back off so we don't spin.
                    error!(error = %e, "repo-ops drain tick failed");
                    sleep(Duration::from_secs(IDLE_POLL_SECS)).await;
                }
            }
        }
    });
}

/// Pump a single claim from the queue. Returns the number of ops
/// processed (0 when the queue is empty).
///
/// Exposed for integration tests that want to drive the drain
/// deterministically instead of spawning the task.
pub async fn drain_once(db: &dyn MetadataBackend, s3: &S3ObjectBackend) -> anyhow::Result<usize> {
    let Some(op) = db.claim_next_repo_op(VISIBILITY_SECS)? else {
        return Ok(0);
    };

    let op_id = op.id;
    let attempts = op.attempts;
    let result: anyhow::Result<usize> = match op.op_type.as_str() {
        "delete" => s3.drain_delete_repo(&op.repo).await,
        "rename" => {
            let Some(new_repo) = op.new_repo.as_deref() else {
                // Corrupt row — a rename op without a destination
                // can't succeed; mark it failed so ops can manually
                // clear it rather than looping forever.
                let err = "rename op has no new_repo; fix DB or drop row";
                warn!(op_id, repo = %op.repo, "{}", err);
                db.fail_repo_op(op_id, err, BACKOFF_CAP_SECS)?;
                return Ok(1);
            };
            s3.drain_rename_repo(&op.repo, new_repo).await
        }
        other => {
            // Unknown op type — same posture as above.
            let err = format!("unknown op_type '{other}'");
            warn!(op_id, op_type = %other, "{}", err);
            db.fail_repo_op(op_id, &err, BACKOFF_CAP_SECS)?;
            return Ok(1);
        }
    };

    match result {
        Ok(n) => {
            info!(
                op_id,
                op_type = %op.op_type,
                repo = %op.repo,
                new_repo = ?op.new_repo,
                keys = n,
                attempts,
                "repo-ops drain completed op"
            );
            db.complete_repo_op(op_id)?;
        }
        Err(e) => {
            let delay = backoff_for_attempts(attempts);
            let msg = format!("{e:#}");
            warn!(
                op_id,
                op_type = %op.op_type,
                repo = %op.repo,
                attempts,
                retry_in_secs = delay,
                error = %msg,
                "repo-ops drain op failed"
            );
            db.fail_repo_op(op_id, &msg, delay)?;
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::backoff_for_attempts;

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff_for_attempts(1), 30);
        assert_eq!(backoff_for_attempts(2), 60);
        assert_eq!(backoff_for_attempts(3), 120);
        assert_eq!(backoff_for_attempts(4), 240);
        assert_eq!(backoff_for_attempts(5), 480);
        // Cap kicks in at 600.
        assert_eq!(backoff_for_attempts(6), 600);
        assert_eq!(backoff_for_attempts(100), 600);
    }

    #[test]
    fn backoff_handles_zero_and_negative() {
        // claim_next_repo_op always bumps attempts to >=1, so 0 is
        // defensive. Should still produce the minimum delay.
        assert_eq!(backoff_for_attempts(0), 30);
        assert_eq!(backoff_for_attempts(-5), 30);
    }
}
