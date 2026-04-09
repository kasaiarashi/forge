// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! `forge login` — authenticate against a forge server and store a credential.
//!
//! Two modes:
//!
//! 1. **`--token <pat>`** — caller already has a PAT (e.g. created in the web
//!    UI). We just verify it via `WhoAmI` and save it. No password prompt.
//!
//! 2. **Interactive** — prompts for username + password, calls `Login` to get
//!    a session token, then immediately mints a long-lived PAT named
//!    `cli-<hostname>` with `repo:read` + `repo:write` scopes and stores
//!    *that*. Sessions are short-lived and meant for browsers; the CLI wants
//!    something that survives across the box rebooting.
//!
//! After saving, prints which backend was used so the user knows whether
//! their credential lives in the OS keychain or `~/.forge/credentials`.

use anyhow::{anyhow, bail, Context, Result};
use forge_proto::forge::{CreatePatRequest, LoginRequest, WhoAmIRequest};

use crate::client;
use crate::credentials::{self, Credential};

pub fn run(
    server: Option<String>,
    token: Option<String>,
    username: Option<String>,
    password: Option<String>,
) -> Result<()> {
    let server_url = resolve_server_url(server)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(login_async(server_url, token, username, password))
}

async fn login_async(
    server_url: String,
    token: Option<String>,
    username: Option<String>,
    password: Option<String>,
) -> Result<()> {
    if let Some(pat) = token {
        return login_with_token(&server_url, pat).await;
    }
    login_interactive(&server_url, username, password).await
}

async fn login_with_token(server_url: &str, token: String) -> Result<()> {
    if token.is_empty() {
        bail!("--token cannot be empty");
    }
    // Save first so the WhoAmI call goes out with the new token attached.
    let cred = Credential {
        user: String::new(),
        token: token.clone(),
    };
    credentials::save(server_url, &cred)?;

    // Verify by calling WhoAmI.
    let mut auth = client::connect_auth(server_url).await?;
    let resp = auth
        .who_am_i(WhoAmIRequest {})
        .await
        .with_context(|| "WhoAmI verification failed — is the token correct?")?
        .into_inner();
    if !resp.authenticated {
        // Roll back the bad save so the user isn't left with a broken creds file.
        let _ = credentials::delete(server_url);
        bail!("token rejected by server");
    }
    let user = resp
        .user
        .ok_or_else(|| anyhow!("server returned no user info"))?;

    // Re-save with the username from WhoAmI so the listing UX is nicer.
    let cred = Credential {
        user: user.username.clone(),
        token,
    };
    let backend = credentials::save(server_url, &cred)?;
    println!("Logged in to {} as {}", server_url, user.username);
    println!("Token stored in {backend}");
    Ok(())
}

async fn login_interactive(
    server_url: &str,
    username_arg: Option<String>,
    password_arg: Option<String>,
) -> Result<()> {
    use std::io::Write;

    let username = match username_arg {
        Some(u) if !u.is_empty() => u,
        _ => {
            print!("Username: ");
            std::io::stdout().flush()?;
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf)?;
            buf.trim().to_string()
        }
    };
    if username.is_empty() {
        bail!("username is required");
    }

    let password = match password_arg {
        Some(p) if !p.is_empty() => p,
        _ => rpassword::prompt_password("Password: ")?,
    };
    if password.is_empty() {
        bail!("password is required");
    }

    // Step 1: log in with username/password to get a session token.
    let mut auth = client::connect_auth(server_url).await?;
    let login_resp = auth
        .login(LoginRequest {
            username: username.clone(),
            password,
            user_agent: format!("forge-cli/{}", env!("CARGO_PKG_VERSION")),
            ip: String::new(),
        })
        .await
        .map_err(|e| anyhow!("login failed: {}", e.message()))?
        .into_inner();

    let session_token = login_resp.session_token;
    let user = login_resp
        .user
        .ok_or_else(|| anyhow!("server returned no user info"))?;

    // Step 2: with that session, mint a long-lived PAT for this CLI box and
    // store *that* — sessions expire after 24h and aren't meant for headless
    // tools.
    let session_cred = Credential {
        user: user.username.clone(),
        token: session_token,
    };
    credentials::save(server_url, &session_cred)?;
    let mut auth = client::connect_auth(server_url).await?;
    let host = whoami::fallible::hostname().unwrap_or_else(|_| "cli".to_string());
    let pat_name = format!("cli-{host}");
    let pat_resp = auth
        .create_personal_access_token(CreatePatRequest {
            name: pat_name.clone(),
            scopes: vec![
                "repo:read".to_string(),
                "repo:write".to_string(),
            ],
            expires_at: 0, // never
        })
        .await
        .map_err(|e| anyhow!("PAT mint failed: {}", e.message()))?
        .into_inner();

    let cred = Credential {
        user: user.username.clone(),
        token: pat_resp.plaintext_token,
    };
    let backend = credentials::save(server_url, &cred)?;

    // Best-effort: revoke the short-lived session now that we have a PAT.
    let _ = auth
        .logout(forge_proto::forge::LogoutRequest {})
        .await;

    println!("Logged in to {} as {}", server_url, user.username);
    println!("Created PAT '{pat_name}' with scopes repo:read, repo:write");
    println!("Token stored in {backend}");
    Ok(())
}

/// Resolve the server URL from `--server`, the workspace's default remote,
/// or an interactive prompt as a last resort.
pub(crate) fn resolve_server_url(server: Option<String>) -> Result<String> {
    if let Some(s) = server {
        if !s.is_empty() {
            return Ok(s);
        }
    }
    // Try the workspace's default remote.
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(ws) = forge_core::workspace::Workspace::discover(&cwd) {
            if let Ok(config) = ws.config() {
                if let Some(url) = config.default_remote_url() {
                    return Ok(url.to_string());
                }
            }
        }
    }
    // Last resort: prompt.
    use std::io::Write;
    print!("Server URL: ");
    std::io::stdout().flush()?;
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf)?;
    let url = buf.trim().to_string();
    if url.is_empty() {
        bail!("server URL is required (pass --server or run inside a forge workspace)");
    }
    Ok(url)
}
