// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! `forge workflow list|trigger|create|delete|enable|disable`.

use anyhow::{bail, Context, Result};
use forge_core::workspace::Workspace;
use forge_proto::forge::*;
use serde_json::json;

fn remote(ws: &Workspace) -> Result<(String, String)> {
    let config = ws.config()?;
    let server_url = config
        .default_remote_url()
        .ok_or_else(|| anyhow::anyhow!("No remote configured. Use: forge remote add origin <url>"))?
        .to_string();
    let repo = if config.repo.is_empty() {
        "default".into()
    } else {
        config.repo.clone()
    };
    Ok((server_url, repo))
}

pub fn list(json_out: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let (server_url, repo) = remote(&ws)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = crate::client::connect_forge(&server_url).await?;
        let resp = client
            .list_workflows(ListWorkflowsRequest { repo })
            .await?
            .into_inner();
        if json_out {
            println!("{}", serde_json::to_string_pretty(&resp.workflows)?);
        } else if resp.workflows.is_empty() {
            println!("No workflows configured.");
        } else {
            println!("{:<24} {:<8} {}", "NAME", "STATE", "ID");
            println!("{}", "-".repeat(48));
            for w in &resp.workflows {
                let state = if w.enabled { "enabled" } else { "disabled" };
                println!("{:<24} {:<8} {}", w.name, state, w.id);
            }
        }
        Ok::<(), anyhow::Error>(())
    })
}

pub fn create(name: &str, file: &str, json_out: bool) -> Result<()> {
    let yaml =
        std::fs::read_to_string(file).with_context(|| format!("read workflow YAML from {file}"))?;
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let (server_url, repo) = remote(&ws)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = crate::client::connect_forge_write(&server_url).await?;
        let resp = client
            .create_workflow(CreateWorkflowRequest {
                repo,
                name: name.to_string(),
                yaml,
            })
            .await?
            .into_inner();
        if !resp.success {
            bail!("{}", resp.error);
        }
        if json_out {
            println!("{}", json!({ "ok": true, "id": resp.id }));
        } else {
            println!("Created workflow '{}' (id {})", name, resp.id);
        }
        Ok::<(), anyhow::Error>(())
    })
}

pub fn delete(id: i64, json_out: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let (server_url, _repo) = remote(&ws)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = crate::client::connect_forge_write(&server_url).await?;
        let resp = client
            .delete_workflow(DeleteWorkflowRequest { id })
            .await?
            .into_inner();
        if !resp.success {
            bail!("{}", resp.error);
        }
        if json_out {
            println!("{}", json!({ "ok": true, "id": id }));
        } else {
            println!("Deleted workflow {}", id);
        }
        Ok::<(), anyhow::Error>(())
    })
}

pub fn set_enabled(id: i64, enabled: bool, json_out: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let (server_url, _repo) = remote(&ws)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = crate::client::connect_forge_write(&server_url).await?;
        // UpdateWorkflow re-submits the full definition; keep name + yaml
        // unchanged by reading them back first.
        let list = client
            .list_workflows(ListWorkflowsRequest {
                repo: String::new(),
            })
            .await
            .ok();
        let (name, yaml) = list
            .and_then(|r| {
                r.into_inner()
                    .workflows
                    .into_iter()
                    .find(|w| w.id == id)
                    .map(|w| (w.name, w.yaml))
            })
            .unwrap_or_default();
        let resp = client
            .update_workflow(UpdateWorkflowRequest {
                id,
                name,
                yaml,
                enabled,
            })
            .await?
            .into_inner();
        if !resp.success {
            bail!("{}", resp.error);
        }
        if json_out {
            println!("{}", json!({ "ok": true, "id": id, "enabled": enabled }));
        } else {
            println!(
                "{} workflow {}",
                if enabled { "Enabled" } else { "Disabled" },
                id
            );
        }
        Ok::<(), anyhow::Error>(())
    })
}

pub fn trigger(workflow_id: i64, ref_name: Option<String>, json_out: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let (server_url, repo) = remote(&ws)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = crate::client::connect_forge_write(&server_url).await?;
        let _ = repo;
        let resp = client
            .trigger_workflow(TriggerWorkflowRequest {
                workflow_id,
                ref_name: ref_name.unwrap_or_default(),
                triggered_by: String::new(),
            })
            .await?
            .into_inner();
        if !resp.success {
            bail!("{}", resp.error);
        }
        if json_out {
            println!("{}", json!({ "ok": true, "run_id": resp.run_id }));
        } else {
            println!("Queued run {} for workflow {}", resp.run_id, workflow_id);
        }
        Ok::<(), anyhow::Error>(())
    })
}
