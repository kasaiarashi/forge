// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{bail, Result};
use forge_core::workspace::Workspace;
use forge_proto::forge::*;
use serde_json::json;

pub fn run(path: String, force: bool, json: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let config = ws.config()?;

    let server_url = config
        .default_remote_url()
        .ok_or_else(|| anyhow::anyhow!("No remote configured. Use: forge remote add origin <url>"))?
        .to_string();

    let rel_path = path.replace('\\', "/");

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = crate::client::connect_forge(&server_url).await?;

        let resp = client
            .release_lock(UnlockRequest {
                repo: if config.repo.is_empty() { "default".into() } else { config.repo.clone() },
                path: rel_path.clone(),
                owner: config.user.name.clone(),
                force,
            })
            .await?
            .into_inner();

        if resp.success {
            if json {
                println!("{}", serde_json::to_string_pretty(&json!({
                    "ok": true,
                    "path": rel_path,
                }))?);
            } else {
                println!("\x1b[32mUnlocked:\x1b[0m {}", rel_path);
            }
        } else {
            let msg = if resp.error.is_empty() {
                "lock not found or owned by another user".to_string()
            } else {
                resp.error
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&json!({
                    "ok": false,
                    "error": msg,
                    "path": rel_path,
                }))?);
            } else {
                bail!("Failed to unlock '{}': {}", rel_path, msg);
            }
        }

        Ok(())
    })
}
