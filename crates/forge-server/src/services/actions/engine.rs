// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Workflow execution engine. Processes queued runs sequentially.

use anyhow::Result;
use forge_core::hash::ForgeHash;
use forge_core::store::object_store::ObjectStore;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::config::ServerConfig;
use crate::storage::db::MetadataDb;
use crate::storage::fs::FsStorage;

use super::workspace;
use super::yaml::WorkflowDef;

/// Start the workflow engine background task.
/// Returns a sender to queue run IDs for execution.
pub fn start(
    config: &ServerConfig,
    db: Arc<MetadataDb>,
    fs: Arc<FsStorage>,
) -> mpsc::Sender<i64> {
    let (tx, rx) = mpsc::channel::<i64>(64);

    let workspaces_path = config.resolved_workspaces_path();
    let artifacts_path = config.resolved_artifacts_path();

    std::fs::create_dir_all(&workspaces_path).ok();
    std::fs::create_dir_all(&artifacts_path).ok();

    tokio::spawn(run_loop(rx, db, fs, workspaces_path, artifacts_path));
    tx
}

async fn run_loop(
    mut rx: mpsc::Receiver<i64>,
    db: Arc<MetadataDb>,
    fs: Arc<FsStorage>,
    workspaces_path: PathBuf,
    artifacts_path: PathBuf,
) {
    while let Some(run_id) = rx.recv().await {
        if let Err(e) = execute_run(run_id, &db, &fs, &workspaces_path, &artifacts_path).await {
            error!("Workflow run {} failed: {}", run_id, e);
            let _ = db.update_run_status(run_id, "failure");
        }
    }
}

async fn execute_run(
    run_id: i64,
    db: &MetadataDb,
    fs: &FsStorage,
    workspaces_path: &PathBuf,
    artifacts_path: &PathBuf,
) -> Result<()> {
    let run = db.get_run(run_id)?
        .ok_or_else(|| anyhow::anyhow!("Run {} not found", run_id))?;

    // Check if cancelled before starting.
    if run.status == "cancelled" {
        return Ok(());
    }

    let workflow = db.get_workflow(run.workflow_id)?
        .ok_or_else(|| anyhow::anyhow!("Workflow {} not found", run.workflow_id))?;

    let def = WorkflowDef::parse(&workflow.yaml)?;
    info!("Starting run {} for workflow '{}' on {}", run_id, def.name, run.repo);

    db.update_run_status(run_id, "running")?;

    // Checkout the repo to a workspace directory.
    let workspace_dir = workspaces_path.join(format!("run-{}", run_id));
    let commit_hash = if !run.commit_hash.is_empty() {
        ForgeHash::from_hex(&run.commit_hash)?
    } else {
        ForgeHash::ZERO
    };

    let store = fs.repo_store(&run.repo);
    let object_store = ObjectStore::new(store.root().to_path_buf());

    let has_checkout = !commit_hash.is_zero();
    if has_checkout {
        workspace::checkout(&object_store, &commit_hash, &workspace_dir)?;
    } else {
        std::fs::create_dir_all(&workspace_dir)?;
    }

    // Execute jobs in definition order (respecting `needs` — for MVP, sequential).
    let mut all_success = true;

    for (_job_key, job_def) in &def.jobs {
        if !all_success {
            break; // Stop on first job failure.
        }

        for (step_idx, step_def) in job_def.steps.iter().enumerate() {
            // Re-check cancellation.
            if let Ok(Some(current)) = db.get_run(run_id) {
                if current.status == "cancelled" {
                    info!("Run {} cancelled, stopping", run_id);
                    workspace::cleanup(&workspace_dir);
                    return Ok(());
                }
            }

            let step_id = db.create_step(run_id, &job_def.name, step_idx as i32, &step_def.name)?;
            db.update_step(step_id, "running", None, "")?;

            if let Some(cmd) = &step_def.run {
                // Execute shell command.
                let result = execute_command(cmd, &workspace_dir, &def.env, &run).await;
                match result {
                    Ok((exit_code, output)) => {
                        let status = if exit_code == 0 { "success" } else { "failure" };
                        db.update_step(step_id, status, Some(exit_code), &output)?;
                        if exit_code != 0 {
                            all_success = false;
                            break;
                        }
                    }
                    Err(e) => {
                        db.update_step(step_id, "failure", Some(-1), &e.to_string())?;
                        all_success = false;
                        break;
                    }
                }
            } else if let Some(artifact_def) = &step_def.artifact {
                // Collect artifact files.
                match collect_artifact(run_id, artifact_def, &workspace_dir, artifacts_path, db) {
                    Ok(msg) => db.update_step(step_id, "success", Some(0), &msg)?,
                    Err(e) => {
                        db.update_step(step_id, "failure", Some(-1), &e.to_string())?;
                        all_success = false;
                        break;
                    }
                }
            } else if let Some(release_def) = &step_def.release {
                // Create release.
                match create_release(run_id, release_def, &run.repo, db) {
                    Ok(msg) => db.update_step(step_id, "success", Some(0), &msg)?,
                    Err(e) => {
                        db.update_step(step_id, "failure", Some(-1), &e.to_string())?;
                        all_success = false;
                        break;
                    }
                }
            } else {
                db.update_step(step_id, "success", Some(0), "No-op step")?;
            }
        }
    }

    let final_status = if all_success { "success" } else { "failure" };
    db.update_run_status(run_id, final_status)?;
    info!("Run {} finished with status: {}", run_id, final_status);

    // Cleanup workspace.
    workspace::cleanup(&workspace_dir);

    Ok(())
}

async fn execute_command(
    cmd: &str,
    working_dir: &std::path::Path,
    env_vars: &std::collections::HashMap<String, String>,
    run: &crate::services::actions::db::RunRecord,
) -> Result<(i32, String)> {
    let (shell, flag) = if cfg!(target_os = "windows") {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };

    let mut command = tokio::process::Command::new(shell);
    command
        .arg(flag)
        .arg(cmd)
        .current_dir(working_dir)
        .env("FORGE_WORKSPACE", working_dir)
        .env("FORGE_REPO", &run.repo)
        .env("FORGE_REF", &run.trigger_ref)
        .env("FORGE_COMMIT", &run.commit_hash)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    for (k, v) in env_vars {
        command.env(k, v);
    }

    let output = command.output().await?;
    let exit_code = output.status.code().unwrap_or(-1);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut log = stdout.to_string();
    if !stderr.is_empty() {
        if !log.is_empty() {
            log.push('\n');
        }
        log.push_str(&stderr);
    }

    Ok((exit_code, log))
}

fn collect_artifact(
    run_id: i64,
    artifact_def: &super::yaml::ArtifactDef,
    workspace_dir: &std::path::Path,
    artifacts_path: &std::path::Path,
    db: &MetadataDb,
) -> Result<String> {
    let dest_dir = artifacts_path.join(format!("{}", run_id)).join(&artifact_def.name);
    std::fs::create_dir_all(&dest_dir)?;

    // Use globset to match files.
    let glob = globset::GlobBuilder::new(&artifact_def.path)
        .literal_separator(false)
        .build()?
        .compile_matcher();

    let mut total_size: i64 = 0;
    let mut file_count = 0;

    for entry in walkdir::WalkDir::new(workspace_dir).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel_path = entry.path().strip_prefix(workspace_dir)?;
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");
        if glob.is_match(&rel_str) {
            let dest = dest_dir.join(rel_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &dest)?;
            total_size += entry.metadata()?.len() as i64;
            file_count += 1;
        }
    }

    let rel_artifact_path = format!("{}/{}", run_id, artifact_def.name);
    db.create_artifact(run_id, &artifact_def.name, &rel_artifact_path, total_size)?;

    Ok(format!("Collected {} files ({} bytes) into artifact '{}'", file_count, total_size, artifact_def.name))
}

fn create_release(
    run_id: i64,
    release_def: &super::yaml::ReleaseDef,
    repo: &str,
    db: &MetadataDb,
) -> Result<String> {
    // Find artifact IDs by name.
    let all_artifacts = db.list_artifacts(run_id)?;
    let artifact_ids: Vec<i64> = release_def
        .artifacts
        .iter()
        .filter_map(|name| all_artifacts.iter().find(|a| a.name == *name).map(|a| a.id))
        .collect();

    let release_id = db.create_release(repo, Some(run_id), &release_def.tag, &release_def.name, &artifact_ids)?;
    Ok(format!("Created release '{}' (tag: {}, id: {}) with {} artifacts",
        release_def.name, release_def.tag, release_id, artifact_ids.len()))
}
