// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! `forge logout` — forget the stored credential for a server.
//!
//! Best-effort: also calls `AuthService::Logout` on the server so any
//! short-lived session token gets revoked. PATs created via `forge login`
//! are NOT auto-revoked here — they're long-lived by design and the user can
//! revoke them explicitly via the web UI or `forge` (future).

use anyhow::Result;

use crate::client;
use crate::commands::login::resolve_server_url;
use crate::credentials;

pub fn run(server: Option<String>) -> Result<()> {
    let server_url = resolve_server_url(server)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(logout_async(server_url))
}

async fn logout_async(server_url: String) -> Result<()> {
    // Resolve web → gRPC so we delete the credential under the same key
    // `forge login` saved it. Otherwise a user who logged in via the web
    // URL and then logged out via the same URL would leave a stale PAT.
    let server_url = crate::url_resolver::resolve(&server_url).await;
    // Best-effort server-side revocation. We swallow errors here because the
    // local credential cleanup is the operation the user actually asked for.
    if credentials::load(&server_url)?.is_some() {
        if let Ok(mut auth) = client::connect_auth(&server_url).await {
            let _ = auth.logout(forge_proto::forge::LogoutRequest {}).await;
        }
    }
    credentials::delete(&server_url)?;
    println!("Forgot credential for {server_url}");
    Ok(())
}
