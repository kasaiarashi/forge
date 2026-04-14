// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Step executor. Parses the workflow YAML handed to us by `ClaimJob`,
//! runs each `run:` step through the local shell, streams stdout/stderr
//! back to the server over `StreamAgentLogs`, then reports final step +
//! run status.

use anyhow::{Context, Result};
use forge_proto::forge::agent_service_client::AgentServiceClient;
use forge_proto::forge::*;
use indexmap::IndexMap;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

use crate::AgentConfig;

#[derive(Debug, Clone, Deserialize)]
struct WorkflowDef {
    #[serde(default)]
    name: String,
    #[serde(default)]
    env: std::collections::HashMap<String, String>,
    jobs: IndexMap<String, JobDef>,
}

#[derive(Debug, Clone, Deserialize)]
struct JobDef {
    #[allow(dead_code)]
    name: String,
    steps: Vec<StepDef>,
}

#[derive(Debug, Clone, Deserialize)]
struct StepDef {
    name: String,
    #[serde(default)]
    run: Option<String>,
    #[serde(rename = "timeout-minutes", default)]
    timeout_minutes: Option<u64>,
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

    let def: WorkflowDef = serde_yaml::from_str(&job.workflow_yaml)
        .context("parse workflow YAML")?;

    let mut all_success = true;
    'jobs: for (_job_key, jd) in &def.jobs {
        for (idx, step) in jd.steps.iter().enumerate() {
            if let Some(cmd) = &step.run {
                let ok = run_command_step(
                    client,
                    &cfg,
                    agent_id,
                    run_id,
                    &jd.name,
                    idx as i32,
                    &step.name,
                    cmd,
                    &workspace,
                    merged_env(&def, &job),
                    step.timeout_minutes.unwrap_or(30),
                )
                .await
                .unwrap_or(false);
                if !ok {
                    all_success = false;
                    break 'jobs;
                }
            } else {
                // v1 agent only handles `run:`. Artifact + release steps
                // remain on the server side until Phase 3 composite
                // actions fold them into `run:` primitives.
                warn!(step = %step.name, "agent skipping non-run step (no local handler)");
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

    // Workspace cleanup; best-effort.
    let _ = std::fs::remove_dir_all(&workspace);
    info!(run_id, status = final_status, "run finished");
    Ok(())
}

fn merged_env(
    def: &WorkflowDef,
    job: &ClaimJobResponse,
) -> std::collections::HashMap<String, String> {
    let mut out = def.env.clone();
    // Server's resolved env wins — it already expanded ${{ secrets.* }}.
    for (k, v) in &job.env {
        out.insert(k.clone(), v.clone());
    }
    out
}

#[allow(clippy::too_many_arguments)]
async fn run_command_step(
    client: &mut AgentServiceClient<tonic::transport::Channel>,
    cfg: &AgentConfig,
    agent_id: i64,
    run_id: i64,
    job_name: &str,
    step_index: i32,
    step_name: &str,
    cmd: &str,
    workspace: &PathBuf,
    env: std::collections::HashMap<String, String>,
    timeout_minutes: u64,
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
        // CREATE_NEW_PROCESS_GROUP
        command.creation_flags(0x00000200);
    }
    for (k, v) in &env {
        command.env(k, v);
    }

    let mut child = command.spawn().context("spawn step")?;
    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");

    // Open a client-streaming log channel. We use a tokio mpsc feeding a
    // stream so the two capture loops can both push chunks.
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
    let tx1 = tx.clone();
    let token1 = cfg.token.clone();
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
    let tx2 = tx.clone();
    let token2 = cfg.token.clone();
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
    let status: ChildStatus = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(s)) => ChildStatus::Ok(s),
        Ok(Err(_)) => ChildStatus::Timeout,
        Err(_) => {
            let _ = child.start_kill();
            warn!(step = %step_name, "step timed out");
            ChildStatus::Timeout
        }
    };

    let _ = stdout_task.await;
    let _ = stderr_task.await;
    drop(tx);
    let _ = log_handle.await;

    let (ok, exit_code) = match status {
        ChildStatus::Ok(s) => (s.success(), s.code().unwrap_or(-1)),
        ChildStatus::Timeout => (false, -1),
    };
    let final_status = if ok { "success" } else { "failure" };
    let log_tail = tail.lock().await.clone();
    let _ = client
        .report_step(ReportStepRequest {
            agent_id,
            token: cfg.token.clone(),
            run_id,
            job_name: job_name.to_string(),
            step_index,
            name: step_name.to_string(),
            status: final_status.to_string(),
            exit_code,
            log_tail,
        })
        .await;

    Ok(ok)
}

// Distinguish a completed child (with exit code) from a timeout. The
// timeout arm reports "failure exit -1" so downstream server code can
// tell the step didn't exit normally.
enum ChildStatus {
    Ok(std::process::ExitStatus),
    Timeout,
}
