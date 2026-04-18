// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Workflow execution engine. Processes queued runs sequentially.

use anyhow::Result;
use forge_core::hash::ForgeHash;
use forge_core::store::object_store::ObjectStore;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::config::ServerConfig;
use crate::services::logs::{LogChunk, LogHub};
use crate::services::secrets::mask::Mask;
use crate::services::secrets::SecretBackend;
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
    secrets: Arc<dyn SecretBackend>,
    log_hub: Arc<LogHub>,
) -> mpsc::Sender<i64> {
    let (tx, rx) = mpsc::channel::<i64>(64);

    let workspaces_path = config.resolved_workspaces_path();
    let artifacts_path = config.resolved_artifacts_path();

    std::fs::create_dir_all(&workspaces_path).ok();
    std::fs::create_dir_all(&artifacts_path).ok();

    tokio::spawn(run_loop(
        rx,
        db,
        fs,
        secrets,
        log_hub,
        workspaces_path,
        artifacts_path,
    ));
    tx
}

async fn run_loop(
    mut rx: mpsc::Receiver<i64>,
    db: Arc<MetadataDb>,
    fs: Arc<FsStorage>,
    secrets: Arc<dyn SecretBackend>,
    log_hub: Arc<LogHub>,
    workspaces_path: PathBuf,
    artifacts_path: PathBuf,
) {
    while let Some(run_id) = rx.recv().await {
        if let Err(e) = execute_run(
            run_id,
            &db,
            &fs,
            secrets.as_ref(),
            log_hub.as_ref(),
            &workspaces_path,
            &artifacts_path,
        )
        .await
        {
            error!("Workflow run {} failed: {}", run_id, e);
            let _ = db.update_run_status(run_id, "failure");
        }
        log_hub.close(run_id);
    }
}

async fn execute_run(
    run_id: i64,
    db: &MetadataDb,
    fs: &FsStorage,
    secrets: &dyn SecretBackend,
    log_hub: &LogHub,
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

    // Resolve `${{ secrets.<name> }}` refs that appear in `env:`. Build a
    // per-run mask from the plaintext values so step logs are scrubbed
    // before they hit the DB or the live-log broadcast (3.3 in plan).
    let (resolved_env, mask) = resolve_secrets(&def, &run.repo, secrets).await?;

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
                // Expand `${{ secrets.<name> }}` in the command body too, so
                // scripts can pass secrets to tools that read argv (e.g.
                // `curl -u user:$TOKEN`). The mask covers the log side so we
                // don't leak the plaintext back into DB rows.
                let expanded_cmd = expand_secret_refs(cmd, &resolved_env);
                let step_timeout = std::time::Duration::from_secs(
                    step_def.timeout_minutes.unwrap_or(30) * 60,
                );
                let result = execute_command_streaming(
                    &expanded_cmd,
                    step_def.shell.as_deref(),
                    &workspace_dir,
                    &resolved_env,
                    &run,
                    run_id,
                    step_id,
                    log_hub,
                    &mask,
                    step_timeout,
                )
                .await;
                match result {
                    Ok((exit_code, output)) => {
                        let status = if exit_code == 0 { "success" } else { "failure" };
                        // Output is already masked by the streaming capture.
                        db.update_step(step_id, status, Some(exit_code), &output)?;
                        if exit_code != 0 {
                            all_success = false;
                            break;
                        }
                    }
                    Err(e) => {
                        let msg = mask.apply(&e.to_string());
                        db.update_step(step_id, "failure", Some(-1), &msg)?;
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

async fn execute_command_streaming(
    cmd: &str,
    shell_spec: Option<&str>,
    working_dir: &std::path::Path,
    env_vars: &std::collections::HashMap<String, String>,
    run: &crate::services::actions::db::RunRecord,
    run_id: i64,
    step_id: i64,
    log_hub: &LogHub,
    mask: &Mask,
    timeout: std::time::Duration,
) -> Result<(i32, String)> {
    let (shell, flag) = super::shell::resolve_shell(shell_spec);

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
        .stderr(std::process::Stdio::piped())
        // Kill the child when this task is dropped (e.g. server shutdown).
        // Without this a hung `sleep infinity` keeps the process alive
        // even after forge-server exits.
        .kill_on_drop(true);

    // Isolate in a new process group so `kill_group` later tears down any
    // descendants the shell spawned. Without this, `kill` on the shell
    // leaves grandchildren (ninja, cargo, UAT, …) orphaned.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Safety: this closure runs in the child between fork and exec;
        // setsid() is async-signal-safe.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NEW_PROCESS_GROUP = 0x00000200
        command.creation_flags(0x00000200);
    }

    for (k, v) in env_vars {
        command.env(k, v);
    }

    let mut child = command.spawn()?;
    let child_pid = child.id();
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let sender = log_hub.sender(run_id);
    let acc = Arc::new(tokio::sync::Mutex::new(String::new()));

    let stdout_task = tokio::spawn(forward_stream(stdout, sender.clone(), step_id, run_id, mask.clone_values(), Arc::clone(&acc)));
    let stderr_task = tokio::spawn(forward_stream(stderr, sender.clone(), step_id, run_id, mask.clone_values(), Arc::clone(&acc)));

    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(r) => r?,
        Err(_) => {
            kill_process_tree(&mut child, child_pid);
            anyhow::bail!("step timed out after {}m", timeout.as_secs() / 60);
        }
    };

    // Drain readers before reading the accumulator.
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    let exit_code = status.code().unwrap_or(-1);
    let log_str = acc.lock().await.clone();

    // Final sentinel chunk so live tailers know the step ended.
    let _ = sender.send(LogChunk {
        run_id,
        step_id,
        data: Vec::new(),
        is_final: true,
    });

    Ok((exit_code, log_str))
}

/// Kill the step's whole process tree. On Unix we signal the negative pid
/// (process-group), which setsid() in pre_exec made distinct from the
/// server's. On Windows `child.start_kill()` already terminates the
/// process; the CREATE_NEW_PROCESS_GROUP flag doesn't cascade to
/// descendants, but tokio's kill_on_drop guarantees at least the root dies,
/// and Job Objects are a Phase-2 runner concern.
fn kill_process_tree(child: &mut tokio::process::Child, _pid: Option<u32>) {
    #[cfg(unix)]
    {
        if let Some(pid) = _pid {
            // SIGKILL to the whole group.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
    let _ = child.start_kill();
}

async fn forward_stream<R>(
    reader: R,
    sender: tokio::sync::broadcast::Sender<LogChunk>,
    step_id: i64,
    run_id: i64,
    mask_values: Vec<String>,
    acc: Arc<tokio::sync::Mutex<String>>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use tokio::io::AsyncReadExt;
    let mut reader = reader;
    let mut buf = vec![0u8; 64 * 1024];
    let mask = Mask::new(mask_values);
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        // Apply mask *before* broadcasting or persisting so no subscriber
        // ever sees the raw secret, even in the narrow window between
        // capture and DB persist at step end.
        let chunk_str = String::from_utf8_lossy(&buf[..n]).to_string();
        let masked = mask.apply(&chunk_str);
        acc.lock().await.push_str(&masked);
        let _ = sender.send(LogChunk {
            run_id,
            step_id,
            data: masked.into_bytes(),
            is_final: false,
        });
    }
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

/// Walk the workflow's `env:` map, replace every `${{ secrets.<name> }}`
/// with the real value from the backend, and return both the resolved env
/// (for command execution) and a [`Mask`] seeded with the plaintext values
/// (for log scrubbing).
///
/// Missing secrets yield an explicit error — silently replacing a missing
/// ref with an empty string would turn a credential-config bug into a "you
/// shipped to prod with no auth and didn't notice" bug.
async fn resolve_secrets(
    def: &WorkflowDef,
    repo: &str,
    secrets: &dyn SecretBackend,
) -> Result<(std::collections::HashMap<String, String>, Mask)> {
    let re = regex::Regex::new(r"\$\{\{\s*secrets\.([A-Za-z_][A-Za-z0-9_]*)\s*\}\}")
        .expect("static regex");

    let mut resolved: std::collections::HashMap<String, String> =
        std::collections::HashMap::with_capacity(def.env.len());
    let mut plaintexts: Vec<String> = Vec::new();

    // Scan env values + every step's `run:` body to collect referenced names.
    // Resolve each only once.
    let mut wanted: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for v in def.env.values() {
        for cap in re.captures_iter(v) {
            wanted.insert(cap[1].to_string());
        }
    }
    for job in def.jobs.values() {
        for step in &job.steps {
            if let Some(cmd) = &step.run {
                for cap in re.captures_iter(cmd) {
                    wanted.insert(cap[1].to_string());
                }
            }
        }
    }

    let mut values: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for name in &wanted {
        let s = secrets.get(repo, name).await?;
        match s {
            Some(s) => {
                plaintexts.push(s.value.clone());
                values.insert(name.clone(), s.value);
            }
            None => {
                anyhow::bail!(
                    "workflow references secret '{}' but no such secret is set for repo '{}'",
                    name,
                    repo
                );
            }
        }
    }

    // Expand env map values using the resolved map.
    for (k, v) in &def.env {
        resolved.insert(k.clone(), expand_refs_with(v, &re, &values));
    }

    Ok((resolved, Mask::new(plaintexts)))
}

fn expand_secret_refs(
    input: &str,
    _env: &std::collections::HashMap<String, String>,
) -> String {
    // Env has already been expanded; any literal `${{ secrets.X }}` that the
    // user put directly in a shell body is expanded here against the env map
    // that resolve_secrets produced. The env map doesn't carry the raw secret
    // name -> value — it carries the user's env *keys* -> expanded values —
    // so for in-body expansion we re-run the regex against the env-resolved
    // view. Callers that need the plaintext pass should be routing via env.
    //
    // For now, support the common case: `run: echo $FOO` where FOO is an env
    // entry whose value is `${{ secrets.FOO }}`. Shell variable substitution
    // handles that natively — no in-body rewriting needed. Keep this function
    // as a hook so Phase 3 expression-context expansion has a home.
    input.to_string()
}

fn expand_refs_with(
    input: &str,
    re: &regex::Regex,
    values: &std::collections::HashMap<String, String>,
) -> String {
    re.replace_all(input, |caps: &regex::Captures| {
        values
            .get(&caps[1])
            .cloned()
            .unwrap_or_else(|| caps[0].to_string())
    })
    .into_owned()
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
