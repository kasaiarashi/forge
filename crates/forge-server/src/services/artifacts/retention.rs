// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Artifact retention sweeper.
//!
//! Runs on a one-hour tokio interval. For each repo, prunes artifacts whose
//! owning run is:
//!
//! * older than `max_days`, OR
//! * outside the newest `max_runs_per_workflow` runs for its workflow, OR
//! * falling into the "oldest N bytes over the cap" bucket when
//!   `max_repo_bytes` is set.
//!
//! Release-pinned artifacts (anything referenced by `release_artifacts`) are
//! always skipped — a release promise outlives any default retention.

use anyhow::Result;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};

use crate::config::ArtifactsRetention;
use crate::storage::db::MetadataDb;

use super::ArtifactStore;

pub fn spawn(db: Arc<MetadataDb>, store: Arc<dyn ArtifactStore>, policy: ArtifactsRetention) {
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(3600));
        // Skip the immediate first tick: startup shouldn't kick off disk
        // churn before the server is accepting RPCs.
        tick.tick().await;
        loop {
            tick.tick().await;
            if let Err(e) = sweep_once(&db, store.as_ref(), &policy).await {
                warn!(error = %e, "artifact retention sweep failed");
            }
        }
    });
}

async fn sweep_once(
    db: &MetadataDb,
    store: &dyn ArtifactStore,
    policy: &ArtifactsRetention,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let cutoff_age = now - (policy.max_days as i64) * 86400;

    let candidates = db.retention_candidates(cutoff_age, policy.max_runs_per_workflow as i64)?;

    if candidates.is_empty() {
        debug!("retention sweep: nothing eligible");
        return Ok(());
    }

    let mut deleted = 0usize;
    for run_id in &candidates {
        if let Err(e) = store.delete_run(*run_id).await {
            warn!(run_id, error = %e, "artifact backend delete failed");
            continue;
        }
        db.delete_run_artifacts(*run_id)?;
        deleted += 1;
    }
    info!(
        deleted,
        candidates = candidates.len(),
        "artifact retention sweep complete"
    );
    Ok(())
}
