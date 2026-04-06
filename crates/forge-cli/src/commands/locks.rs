// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::Result;
use forge_core::workspace::Workspace;
use forge_proto::forge::forge_service_client::ForgeServiceClient;
use forge_proto::forge::*;

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let config = ws.config()?;

    let server_url = config
        .server_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No server configured"))?
        .to_string();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = ForgeServiceClient::connect(server_url).await?;

        let resp = client
            .list_locks(ListLocksRequest {
                path_prefix: String::new(),
                owner: String::new(),
            })
            .await?
            .into_inner();

        if resp.locks.is_empty() {
            println!("No active locks.");
        } else {
            println!("{:<40} {:<20} {}", "PATH", "OWNER", "SINCE");
            println!("{}", "-".repeat(80));
            for lock in &resp.locks {
                let since = chrono::DateTime::from_timestamp(lock.created_at, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_else(|| "unknown".into());
                println!("{:<40} {:<20} {}", lock.path, lock.owner, since);
            }
        }

        Ok(())
    })
}
