// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

use anyhow::{bail, Result};
use forge_core::workspace::Workspace;
use forge_proto::forge::*;
use serde_json::json;

pub fn run(path: String, reason: Option<String>, json: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let config = ws.config()?;

    let server_url = config
        .default_remote_url()
        .ok_or_else(|| anyhow::anyhow!("No remote configured. Use: forge remote add origin <url>"))?
        .to_string();

    // Normalize path.
    let rel_path = path.replace('\\', "/");

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = crate::client::connect_forge_write(&server_url).await?;

        let resp = client
            .acquire_lock(LockRequest {
                repo: if config.repo.is_empty() {
                    "default".into()
                } else {
                    config.repo.clone()
                },
                path: rel_path.clone(),
                owner: config.user.name.clone(),
                workspace_id: config.workspace_id.clone(),
                reason: reason.unwrap_or_default(),
            })
            .await?
            .into_inner();

        if resp.granted {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "ok": true,
                        "path": rel_path,
                        "owner": config.user.name,
                    }))?
                );
            } else {
                println!("\x1b[32mLocked:\x1b[0m {}", rel_path);
            }
        } else if let Some(lock) = resp.existing_lock {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "ok": false,
                        "error": format!("already locked by '{}'", lock.owner),
                        "path": rel_path,
                        "owner": lock.owner,
                        "created_at": lock.created_at,
                    }))?
                );
            } else {
                bail!(
                    "File '{}' is already locked by '{}' (since {})",
                    rel_path,
                    lock.owner,
                    chrono::DateTime::from_timestamp(lock.created_at, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                        .unwrap_or_else(|| "unknown".into())
                );
            }
        } else {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "ok": false,
                        "error": "server denied without details",
                        "path": rel_path,
                    }))?
                );
            } else {
                bail!(
                    "Failed to acquire lock on '{}': server denied without details",
                    rel_path
                );
            }
        }

        Ok(())
    })
}
