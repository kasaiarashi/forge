// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Agent heartbeat sweeper.
//!
//! `RegisterAgent` hands each agent a `heartbeat_seconds` interval (15s)
//! and this sweeper requeues any `running` workflow run whose owning agent
//! has not checked in for `STALE_AFTER_SECS`. Without it, a crashed agent
//! would hold its claim forever and the run would never finish or retry.

use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};

use crate::storage::db::MetadataDb;

/// Runs whose agent hasn't heartbeat for this long are considered dead
/// and their claim is released. 8× the 15s heartbeat interval gives a
/// generous window for transient network blips without letting real
/// crashes hold work hostage.
const STALE_AFTER_SECS: i64 = 120;

/// How often the sweep runs. Cheap — a single indexed SELECT.
const SWEEP_INTERVAL_SECS: u64 = 30;

pub fn spawn(db: Arc<MetadataDb>) {
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(SWEEP_INTERVAL_SECS));
        tick.tick().await;
        loop {
            tick.tick().await;
            let cutoff = chrono::Utc::now().timestamp() - STALE_AFTER_SECS;
            match db.requeue_stale_runs(cutoff) {
                Ok(0) => debug!("agent sweep: no stale runs"),
                Ok(n) => info!(requeued = n, "agent sweep: requeued stale runs"),
                Err(e) => warn!(error = %e, "agent sweep failed"),
            }
        }
    });
}
