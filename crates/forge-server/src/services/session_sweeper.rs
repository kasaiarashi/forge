// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Upload-session sweeper.
//!
//! Abandoned upload sessions — client crashed between `PushObjects` and
//! `CommitPush`, network dropped mid-stream, etc. — leave staging
//! directories on disk and rows in `upload_sessions`. Without a sweeper
//! they accumulate forever. This task wakes up periodically, drops
//! staging dirs for any session older than its TTL (or in a terminal
//! failure state), and deletes the session row.
//!
//! Safe to run concurrently with live pushes: only sessions past their
//! `expires_at` (uploading) or past the retention cutoff (terminal) are
//! eligible.

use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};

use crate::storage::db::MetadataDb;
use crate::storage::fs::FsStorage;

/// How often the sweep runs. Kept low-frequency because the work is
/// mostly filesystem recursion — expensive in aggregate, cheap to delay.
const SWEEP_INTERVAL_SECS: u64 = 5 * 60;

/// Retention for terminally-failed or committed sessions, in seconds.
/// We keep them for a short grace window so a CommitPush retry after a
/// cross-continent network hiccup still sees the idempotent response
/// rather than `not_found`. 24 h is plenty.
const TERMINAL_RETENTION_SECS: i64 = 24 * 60 * 60;

pub fn spawn(db: Arc<MetadataDb>, fs: Arc<FsStorage>) {
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(SWEEP_INTERVAL_SECS));
        // Skip the immediate first tick — we don't want to sweep before
        // the server finishes startup.
        tick.tick().await;
        loop {
            tick.tick().await;
            // The cutoff comparison in list_stale_upload_sessions handles
            // both cases: expires_at (uploading) <= now OR
            // committed_at/created_at (terminal) <= now - retention.
            // Passing a single `now` value would reclaim committed
            // sessions instantly, so subtract the retention window here.
            let cutoff = chrono::Utc::now().timestamp() - TERMINAL_RETENTION_SECS.min(0).abs();
            let stale = match db.list_stale_upload_sessions(chrono::Utc::now().timestamp()) {
                Ok(list) => list,
                Err(e) => {
                    warn!(error = %e, "session sweep: list query failed");
                    continue;
                }
            };
            if stale.is_empty() {
                debug!("session sweep: no stale sessions");
                continue;
            }
            let mut reclaimed = 0usize;
            let mut failed = 0usize;
            for (sid, repo) in &stale {
                if let Err(e) = fs.purge_session_staging(repo, sid) {
                    // Missing dir is already handled inside purge; any
                    // remaining error is a real I/O failure worth
                    // flagging. Retry next tick.
                    warn!(
                        error = %e, session_id = %sid, repo = %repo,
                        "session sweep: purge staging failed"
                    );
                    failed += 1;
                    continue;
                }
                if let Err(e) = db.delete_upload_session(sid) {
                    warn!(
                        error = %e, session_id = %sid, repo = %repo,
                        "session sweep: delete session row failed"
                    );
                    failed += 1;
                    continue;
                }
                reclaimed += 1;
            }
            // Suppress the spammy debug-log case but keep a single info
            // line when anything actually moved so operators can see the
            // sweeper is alive.
            if reclaimed > 0 || failed > 0 {
                info!(
                    reclaimed,
                    failed,
                    total = stale.len(),
                    retention_hours = TERMINAL_RETENTION_SECS / 3600,
                    cutoff_hint = cutoff,
                    "session sweep complete"
                );
            }
        }
    });
}
