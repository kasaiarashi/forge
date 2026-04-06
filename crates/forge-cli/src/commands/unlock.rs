// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{bail, Result};
use forge_core::workspace::Workspace;
use forge_proto::forge::forge_service_client::ForgeServiceClient;
use forge_proto::forge::*;

pub fn run(path: String, force: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let config = ws.config()?;

    let server_url = config
        .server_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No server configured"))?
        .to_string();

    let rel_path = path.replace('\\', "/");

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = ForgeServiceClient::connect(server_url).await?;

        let resp = client
            .release_lock(UnlockRequest {
                path: rel_path.clone(),
                owner: config.user.name.clone(),
                force,
            })
            .await?
            .into_inner();

        if resp.success {
            println!("Unlocked: {}", rel_path);
        } else {
            bail!("Failed to unlock '{}': {}", rel_path, resp.error);
        }

        Ok(())
    })
}
