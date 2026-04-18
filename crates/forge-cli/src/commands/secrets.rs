// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! `forge secrets set|delete|list`.
//!
//! Secret **values** cannot be read back — the server exposes only
//! create/update/delete and key listing, so the CLI only speaks the same
//! vocabulary. Values are sent over the existing TLS channel the CLI
//! already uses for every other command.

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

pub fn set(key: &str, value: Option<String>, file: Option<String>, json_out: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let (server_url, repo) = remote(&ws)?;

    let value = match (value, file) {
        (Some(v), _) => v,
        (None, Some(p)) => std::fs::read_to_string(&p)
            .with_context(|| format!("read secret file {p}"))?
            .trim_end_matches(['\n', '\r'])
            .to_string(),
        (None, None) => {
            // Interactive prompt. Avoid echoing the secret.
            rpassword::prompt_password(format!("Enter value for {key}: "))?
        }
    };

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = crate::client::connect_forge(&server_url).await?;
        let resp = client
            .create_secret(CreateSecretRequest {
                repo: repo.clone(),
                key: key.to_string(),
                value,
            })
            .await?
            .into_inner();
        if !resp.success {
            bail!("server rejected secret: {}", resp.error);
        }
        if json_out {
            println!("{}", json!({ "ok": true, "key": key }));
        } else {
            println!("Stored secret '{}'", key);
        }
        Ok::<(), anyhow::Error>(())
    })
}

pub fn delete(key: &str, json_out: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let (server_url, repo) = remote(&ws)?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = crate::client::connect_forge(&server_url).await?;
        let resp = client
            .delete_secret(DeleteSecretRequest {
                repo,
                key: key.to_string(),
            })
            .await?
            .into_inner();
        if !resp.success {
            bail!(
                "{}",
                if resp.error.is_empty() {
                    "delete failed".into()
                } else {
                    resp.error
                }
            );
        }
        if json_out {
            println!("{}", json!({ "ok": true, "key": key }));
        } else {
            println!("Deleted secret '{}'", key);
        }
        Ok::<(), anyhow::Error>(())
    })
}

pub fn list(json_out: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let (server_url, repo) = remote(&ws)?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = crate::client::connect_forge(&server_url).await?;
        let resp = client
            .list_secret_keys(ListSecretKeysRequest { repo })
            .await?
            .into_inner();
        if json_out {
            let arr: Vec<_> = resp
                .secrets
                .iter()
                .map(|s| {
                    json!({
                        "key": s.key,
                        "updated_at": s.updated_at,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr)?);
        } else if resp.secrets.is_empty() {
            println!("No secrets configured.");
        } else {
            println!("{:<32} {}", "KEY", "UPDATED");
            println!("{}", "-".repeat(64));
            for s in &resp.secrets {
                let when = chrono::DateTime::from_timestamp(s.updated_at, 0)
                    .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_else(|| "unknown".into());
                println!("{:<32} {}", s.key, when);
            }
        }
        Ok::<(), anyhow::Error>(())
    })
}
