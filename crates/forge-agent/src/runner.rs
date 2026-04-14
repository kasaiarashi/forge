// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Step executor. Handles `run:` steps directly, `uses: @builtin/*` via
//! the primitive dispatcher, and `uses: <owner>/<name>@<version>` by
//! fetching the composite action YAML from the server and inline-expanding
//! its steps. Expression expansion handles `${{ inputs.* }}` and
//! `${{ steps.<id>.outputs.* }}` scopes; secrets were already expanded
//! server-side into `env:`.

use anyhow::{Context, Result};
use forge_proto::forge::agent_service_client::AgentServiceClient;
use forge_proto::forge::*;
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

use crate::actions::{expand_expr, ActionDef, ComposedStep};
use crate::primitives::dispatch;
use crate::AgentConfig;

#[derive(Debug, Clone, Deserialize)]
struct WorkflowDef {
    #[serde(default)]
    #[allow(dead_code)]
    name: String,
    #[serde(default)]
    env: HashMap<String, String>,
    jobs: IndexMap<String, JobDef>,
}

#[derive(Debug, Clone, Deserialize)]
struct JobDef {
    #[serde(default)]
    name: String,
    steps: Vec<ComposedStep>,
}

pub async fn execute_run(
    client: &mut AgentServiceClient<tonic::transport::Channel>,
    cfg: Arc<AgentConfig>,
    agent_id: i64,
    job: ClaimJobResponse,
) -> Result<()> {
    let run_id = job.run_id;
    let workspace = cfg.work_dir.join(format!("run-{run_id}"));
    std::fs::create_dir_all(&workspace)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&workspace)?.permissions();
        perms.set_mode(0o700);
        let _ = std::fs::set_permissions(&workspace, perms);
    }

    let def: WorkflowDef =
        serde_yaml::from_str(&job.workflow_yaml).context("parse workflow YAML")?;
    let base_env = merged_env(&def, &job);

    let mut all_success = true;
    let mut step_counter: i32 = 0;
    'jobs: for (_job_key, jd) in &def.jobs {
        let job_name = if jd.name.is_empty() {
            _job_key.clone()
        } else {
            jd.name.clone()
        };
        // Per-job shared output map; outer scope has no inputs.
        let mut step_outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
        let empty_inputs: HashMap<String, String> = HashMap::new();

        for step in &jd.steps {
            let ok = execute_step(
                client,
                cfg.as_ref(),
                agent_id,
                run_id,
                &job_name,
                &mut step_counter,
                step,
                &workspace,
                &base_env,
                &empty_inputs,
                &mut step_outputs,
            )
            .await
            .unwrap_or(false);
            if !ok {
                all_success = false;
                break 'jobs;
            }
        }
    }

    let final_status = if all_success { "success" } else { "failure" };
    let _ = client
        .report_run_finished(ReportRunFinishedRequest {
            agent_id,
            token: cfg.token.clone(),
            run_id,
            status: final_status.to_string(),
        })
        .await;
    let _ = std::fs::remove_dir_all(&workspace);
    info!(run_id, status = final_status, "run finished");
    Ok(())
}

fn merged_env(
    def: &WorkflowDef,
    job: &ClaimJobResponse,
) -> HashMap<String, String> {
    let mut out = def.env.clone();
    for (k, v) in &job.env {
        out.insert(k.clone(), v.clone());
    }
    out
}

/// Execute a single step — may be `run:`, a `@builtin/*` primitive, or a
/// composite action that expands into N sub-steps. Recursion depth is
/// implicit and bounded by composite-depth-3 server-side (we trust the
/// server's pre-flight for that; we'd otherwise add a depth counter here).
#[allow(clippy::too_many_arguments)]
async fn execute_step(
    client: &mut AgentServiceClient<tonic::transport::Channel>,
    cfg: &AgentConfig,
    agent_id: i64,
    run_id: i64,
    job_name: &str,
    step_counter: &mut i32,
    step: &ComposedStep,
    workspace: &PathBuf,
    env: &HashMap<String, String>,
    inputs: &HashMap<String, String>,
    step_outputs: &mut HashMap<String, HashMap<String, String>>,
) -> Result<bool> {
    // `run:` takes priority. Expand expressions against the current scope.
    if let Some(cmd) = &step.run {
        let expanded = expand_expr(cmd, inputs, step_outputs);
        return run_shell(
            client,
            cfg,
            agent_id,
            run_id,
            job_name,
            step_counter,
            &step.name,
            &expanded,
            workspace,
            env,
            step.timeout_minutes.unwrap_or(30),
            step.id.as_deref(),
            step_outputs,
        )
        .await;
    }

    // `uses:` — either @builtin/* or a composite.
    let uses = match &step.uses {
        Some(u) => u.clone(),
        None => {
            // No run and no uses — treat as a name-only marker step, success.
            warn!(step = %step.name, "step has neither run nor uses; skipping");
            return Ok(true);
        }
    };

    // Expand `with:` values now so both primitives and composites see
    // the resolved inputs from the outer scope.
    let mut resolved_with: IndexMap<String, String> = IndexMap::new();
    for (k, v) in &step.with {
        resolved_with.insert(k.clone(), expand_expr(v, inputs, step_outputs));
    }

    if let Some(prim) = uses.strip_prefix("@builtin/") {
        let prim_name = format!("@builtin/{}", prim);
        let outcome = match dispatch(&prim_name, &resolved_with) {
            Ok(o) => o,
            Err(e) => {
                // Emit a synthetic failure step so the server-side run
                // record shows where we gave up.
                let _ = client
                    .report_step(ReportStepRequest {
                        agent_id,
                        token: cfg.token.clone(),
                        run_id,
                        job_name: job_name.to_string(),
                        step_index: *step_counter,
                        name: step.name.clone(),
                        status: "failure".into(),
                        exit_code: -1,
                        log_tail: format!("{e}"),
                    })
                    .await;
                *step_counter += 1;
                return Ok(false);
            }
        };
        if let Some(id) = &step.id {
            step_outputs.insert(id.clone(), outcome.outputs.clone());
        }
        match outcome.command {
            None => {
                // Primitive was metadata-only (ue-discover). Log a trivial
                // success step so the server's run log has a breadcrumb.
                let log = outcome
                    .outputs
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let _ = client
                    .report_step(ReportStepRequest {
                        agent_id,
                        token: cfg.token.clone(),
                        run_id,
                        job_name: job_name.to_string(),
                        step_index: *step_counter,
                        name: step.name.clone(),
                        status: "success".into(),
                        exit_code: 0,
                        log_tail: log,
                    })
                    .await;
                *step_counter += 1;
                return Ok(true);
            }
            Some(cmd) => {
                return run_shell(
                    client,
                    cfg,
                    agent_id,
                    run_id,
                    job_name,
                    step_counter,
                    &step.name,
                    &cmd,
                    workspace,
                    env,
                    step.timeout_minutes.unwrap_or(30),
                    step.id.as_deref(),
                    step_outputs,
                )
                .await;
            }
        }
    }

    // Composite action. Split `owner/name@version` → fetch via GetAction.
    let (name, version) = match uses.rsplit_once('@') {
        Some((n, v)) => (n.to_string(), v.to_string()),
        None => (uses.clone(), "v1".to_string()),
    };
    let fetched = client
        .get_action(GetActionRequest {
            agent_id,
            token: cfg.token.clone(),
            name: name.clone(),
            version: version.clone(),
        })
        .await;
    let yaml = match fetched {
        Ok(r) => r.into_inner().yaml,
        Err(e) => {
            warn!(action = %uses, error = %e, "get_action failed");
            let _ = client
                .report_step(ReportStepRequest {
                    agent_id,
                    token: cfg.token.clone(),
                    run_id,
                    job_name: job_name.to_string(),
                    step_index: *step_counter,
                    name: step.name.clone(),
                    status: "failure".into(),
                    exit_code: -1,
                    log_tail: format!("action '{}' not resolvable: {}", uses, e),
                })
                .await;
            *step_counter += 1;
            return Ok(false);
        }
    };
    let action = ActionDef::parse(&yaml)?;
    let action_inputs = action.resolve_inputs(&resolved_with)?;
    // Composite steps execute in a scope where `inputs.*` resolves from
    // the caller's `with:` (with defaults applied), and its own `steps.*`
    // are isolated to the composite so they don't pollute the parent.
    let mut composite_outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
    for inner in &action.steps {
        let ok = Box::pin(execute_step(
            client,
            cfg,
            agent_id,
            run_id,
            job_name,
            step_counter,
            inner,
            workspace,
            env,
            &action_inputs,
            &mut composite_outputs,
        ))
        .await
        .unwrap_or(false);
        if !ok {
            return Ok(false);
        }
    }
    // Export composite's outputs under the outer step's id.
    if let Some(id) = &step.id {
        let mut exported: HashMap<String, String> = HashMap::new();
        for (k, raw_expr) in &action.outputs {
            exported.insert(k.clone(), expand_expr(raw_expr, &action_inputs, &composite_outputs));
        }
        step_outputs.insert(id.clone(), exported);
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn run_shell(
    client: &mut AgentServiceClient<tonic::transport::Channel>,
    cfg: &AgentConfig,
    agent_id: i64,
    run_id: i64,
    job_name: &str,
    step_counter: &mut i32,
    step_name: &str,
    cmd: &str,
    workspace: &PathBuf,
    env: &HashMap<String, String>,
    timeout_minutes: u64,
    step_id_key: Option<&str>,
    step_outputs: &mut HashMap<String, HashMap<String, String>>,
) -> Result<bool> {
    use tokio::io::AsyncReadExt;

    let (shell, flag) = if cfg!(target_os = "windows") {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let mut command = tokio::process::Command::new(shell);
    command
        .arg(flag)
        .arg(cmd)
        .current_dir(workspace)
        .env("FORGE_WORKSPACE", workspace)
        .env("FORGE_RUN_ID", run_id.to_string())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
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
        command.creation_flags(0x00000200);
    }
    for (k, v) in env {
        command.env(k, v);
    }

    let mut child = command.spawn().context("spawn step")?;
    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");

    let (tx, rx) = tokio::sync::mpsc::channel::<AgentLogChunk>(256);
    let log_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let log_handle = {
        let mut client = client.clone();
        tokio::spawn(async move {
            let _ = client.stream_agent_logs(log_stream).await;
        })
    };

    let tail: Arc<tokio::sync::Mutex<String>> =
        Arc::new(tokio::sync::Mutex::new(String::new()));

    let token1 = cfg.token.clone();
    let tx1 = tx.clone();
    let tail1 = Arc::clone(&tail);
    let stdout_task = tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match stdout.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    tail1.lock().await.push_str(&text);
                    let _ = tx1
                        .send(AgentLogChunk {
                            agent_id,
                            token: token1.clone(),
                            run_id,
                            step_id: 0,
                            data: text.into_bytes(),
                            is_final: false,
                        })
                        .await;
                }
            }
        }
    });
    let token2 = cfg.token.clone();
    let tx2 = tx.clone();
    let tail2 = Arc::clone(&tail);
    let stderr_task = tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match stderr.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    tail2.lock().await.push_str(&text);
                    let _ = tx2
                        .send(AgentLogChunk {
                            agent_id,
                            token: token2.clone(),
                            run_id,
                            step_id: 0,
                            data: text.into_bytes(),
                            is_final: false,
                        })
                        .await;
                }
            }
        }
    });

    let timeout = std::time::Duration::from_secs(timeout_minutes * 60);
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(s)) => Some(s),
        Ok(Err(_)) | Err(_) => {
            let _ = child.start_kill();
            None
        }
    };
    let _ = stdout_task.await;
    let _ = stderr_task.await;
    drop(tx);
    let _ = log_handle.await;

    let (ok, exit_code) = match status {
        Some(s) => (s.success(), s.code().unwrap_or(-1)),
        None => (false, -1),
    };

    let log_tail = tail.lock().await.clone();
    let _ = client
        .report_step(ReportStepRequest {
            agent_id,
            token: cfg.token.clone(),
            run_id,
            job_name: job_name.to_string(),
            step_index: *step_counter,
            name: step_name.to_string(),
            status: if ok { "success".into() } else { "failure".into() },
            exit_code,
            log_tail: log_tail.clone(),
        })
        .await;
    *step_counter += 1;

    // Expose `exit_code` + last-line `log` as step outputs so composites
    // can inspect them via ${{ steps.id.outputs.exit_code }}.
    if let Some(id) = step_id_key {
        let mut out = HashMap::new();
        out.insert("exit_code".into(), exit_code.to_string());
        step_outputs.insert(id.to_string(), out);
    }

    Ok(ok)
}
