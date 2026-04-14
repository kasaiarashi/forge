// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! `forge runs list|show|logs|cancel`.

use anyhow::{bail, Result};
use forge_core::workspace::Workspace;
use forge_proto::forge::*;
use serde_json::json;

fn server_url() -> Result<String> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let config = ws.config()?;
    Ok(config
        .default_remote_url()
        .ok_or_else(|| {
            anyhow::anyhow!("No remote configured. Use: forge remote add origin <url>")
        })?
        .to_string())
}

fn repo_name() -> Result<String> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let config = ws.config()?;
    Ok(if config.repo.is_empty() {
        "default".into()
    } else {
        config.repo.clone()
    })
}

pub fn list(workflow_id: i64, limit: i32, json_out: bool) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let url = server_url()?;
        let repo = repo_name()?;
        let mut client = crate::client::connect_forge(&url).await?;
        let resp = client
            .list_workflow_runs(ListWorkflowRunsRequest {
                repo,
                workflow_id,
                limit,
                offset: 0,
            })
            .await?
            .into_inner();
        if json_out {
            println!("{}", serde_json::to_string_pretty(&resp.runs)?);
        } else if resp.runs.is_empty() {
            println!("No runs.");
        } else {
            println!("{:<8} {:<12} {:<20} {}", "ID", "STATUS", "WORKFLOW", "STARTED");
            println!("{}", "-".repeat(64));
            for r in &resp.runs {
                let when = chrono::DateTime::from_timestamp(r.started_at, 0)
                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "queued".into());
                println!(
                    "{:<8} {:<12} {:<20} {}",
                    r.id, r.status, r.workflow_name, when
                );
            }
        }
        Ok::<(), anyhow::Error>(())
    })
}

pub fn show(run_id: i64, json_out: bool) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let url = server_url()?;
        let mut client = crate::client::connect_forge(&url).await?;
        let resp = client
            .get_workflow_run(GetWorkflowRunRequest { run_id })
            .await?
            .into_inner();
        if json_out {
            println!("{}", serde_json::to_string_pretty(&resp)?);
            return Ok(());
        }
        if let Some(run) = &resp.run {
            println!("Run {}  [{}]", run.id, run.status);
            println!("  workflow: {}", run.workflow_name);
            println!("  ref:      {}", run.trigger_ref);
            println!("  commit:   {}", run.commit_hash);
            println!("  by:       {}", run.triggered_by);
        }
        if !resp.steps.is_empty() {
            println!("\nSteps:");
            for s in &resp.steps {
                println!("  [{}] {} ({})", s.status, s.name, s.job_name);
            }
        }
        if !resp.artifacts.is_empty() {
            println!("\nArtifacts:");
            for a in &resp.artifacts {
                println!("  {:<32} {} bytes  (id {})", a.name, a.size_bytes, a.id);
            }
        }
        Ok::<(), anyhow::Error>(())
    })
}

pub fn logs(run_id: i64, step_id: i64, follow: bool, _json_out: bool) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let url = server_url()?;
        let mut client = crate::client::connect_forge(&url).await?;
        let mut stream = client
            .stream_step_logs(StreamStepLogsRequest {
                run_id,
                step_id,
                no_follow: !follow,
            })
            .await?
            .into_inner();
        use tokio_stream::StreamExt as _;
        let mut out = tokio::io::BufWriter::new(tokio::io::stdout());
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => {
                    if chunk.data.is_empty() && chunk.is_final {
                        continue;
                    }
                    use tokio::io::AsyncWriteExt;
                    out.write_all(&chunk.data).await?;
                    out.flush().await?;
                }
                Err(e) => {
                    // Transport-level error: print and bail.
                    bail!("log stream error: {}", e.message());
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    })
}

pub fn cancel(run_id: i64, json_out: bool) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let url = server_url()?;
        let mut client = crate::client::connect_forge(&url).await?;
        let resp = client
            .cancel_workflow_run(CancelWorkflowRunRequest { run_id })
            .await?
            .into_inner();
        if !resp.success {
            bail!("{}", resp.error);
        }
        if json_out {
            println!("{}", json!({ "ok": true, "run_id": run_id }));
        } else {
            println!("Cancelled run {}", run_id);
        }
        Ok::<(), anyhow::Error>(())
    })
}
