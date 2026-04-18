// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Push trigger integration — checks workflows on ref updates.

use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info};

use super::yaml::WorkflowDef;
use crate::storage::db::MetadataDb;

/// Check if any enabled workflows should trigger on a push to the given ref.
/// If so, create runs and queue them for execution.
pub fn check_push_triggers(
    db: &Arc<MetadataDb>,
    engine_tx: &mpsc::Sender<i64>,
    repo: &str,
    ref_name: &str,
    new_hash: &[u8],
) {
    let workflows = match db.get_enabled_workflows_for_repo(repo) {
        Ok(w) => w,
        Err(e) => {
            debug!("Failed to check push triggers for {}: {}", repo, e);
            return;
        }
    };

    let commit_hash = hex::encode(new_hash);

    for workflow in workflows {
        let def = match WorkflowDef::parse(&workflow.yaml) {
            Ok(d) => d,
            Err(_) => continue,
        };

        if !def.matches_push(ref_name) {
            continue;
        }

        info!(
            "Push trigger matched workflow '{}' for {} on {}",
            workflow.name, repo, ref_name
        );

        match db.create_run(repo, workflow.id, "push", ref_name, &commit_hash, "system") {
            Ok(run_id) => {
                let _ = engine_tx.try_send(run_id);
            }
            Err(e) => {
                debug!("Failed to create push-triggered run: {}", e);
            }
        }
    }
}
