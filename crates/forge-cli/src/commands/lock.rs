// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{bail, Result};
use forge_core::workspace::Workspace;
use forge_proto::forge::forge_service_client::ForgeServiceClient;
use forge_proto::forge::*;

pub fn run(path: String, reason: Option<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let config = ws.config()?;

    let server_url = config
        .server_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No server configured. Set server_url in .forge/config.json"))?
        .to_string();

    // Normalize path.
    let rel_path = path.replace('\\', "/");

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = ForgeServiceClient::connect(server_url).await?;

        let resp = client
            .acquire_lock(LockRequest {
                path: rel_path.clone(),
                owner: config.user.name.clone(),
                workspace_id: config.workspace_id.clone(),
                reason: reason.unwrap_or_default(),
            })
            .await?
            .into_inner();

        if resp.granted {
            println!("Locked: {}", rel_path);
        } else if let Some(lock) = resp.existing_lock {
            bail!(
                "File '{}' is locked by {} (since {})",
                rel_path,
                lock.owner,
                chrono::DateTime::from_timestamp(lock.created_at, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_else(|| "unknown".into())
            );
        }

        Ok(())
    })
}
