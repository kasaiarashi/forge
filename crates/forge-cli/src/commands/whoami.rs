// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! `forge whoami` — show the authenticated user for a forge server.

use anyhow::{anyhow, Result};
use forge_proto::forge::WhoAmIRequest;

use crate::client;
use crate::commands::login::resolve_server_url;

pub fn run(server: Option<String>) -> Result<()> {
    let server_url = resolve_server_url(server)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(whoami_async(server_url))
}

async fn whoami_async(server_url: String) -> Result<()> {
    // Same auto-switch web → gRPC as login/logout.
    let server_url = crate::url_resolver::resolve(&server_url).await;
    let mut auth = client::connect_auth(&server_url).await?;
    let resp = auth
        .who_am_i(WhoAmIRequest {})
        .await
        .map_err(|e| anyhow!("{}: {}", server_url, e.message()))?
        .into_inner();

    if !resp.authenticated {
        println!("Not logged in to {server_url}");
        println!("Run: forge login --server {server_url}");
        return Ok(());
    }
    let user = resp
        .user
        .ok_or_else(|| anyhow!("server returned no user info"))?;
    println!("{server_url}");
    println!("  user:   {} ({})", user.username, user.email);
    if user.is_server_admin {
        println!("  role:   server admin");
    }
    if !resp.scopes.is_empty() {
        println!("  scopes: {}", resp.scopes.join(", "));
    }
    Ok(())
}
