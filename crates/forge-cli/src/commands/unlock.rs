// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

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

    let repo = if config.repo.is_empty() {
        "default".to_string()
    } else {
        config.repo.clone()
    };
    // Owner must match the value the server stored — which on a 0.2.5+
    // client is the authenticated username, not `config.user.name`.
    let owner = super::lock::resolve_lock_owner(&server_url, &config.user.name);

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut client = crate::client::connect_forge_write(&server_url).await?;

        // `forge unlock .` — release every lock the caller currently
        // owns in this repo. Mirrors `forge add .`. Without `--force`
        // a lock the server refuses to release is reported but doesn't
        // abort the run, so one stale entry can't strand the rest of
        // the workspace.
        if path == "." {
            let locks = client
                .list_locks(ListLocksRequest {
                    repo: repo.clone(),
                    path_prefix: String::new(),
                    owner: owner.clone(),
                })
                .await?
                .into_inner()
                .locks;

            if locks.is_empty() {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "ok": true,
                            "unlocked": [],
                            "failed": [],
                        }))?
                    );
                } else {
                    println!("No locks owned by '{owner}' to release.");
                }
                return Ok(());
            }

            let mut unlocked: Vec<String> = Vec::new();
            let mut failed: Vec<(String, String)> = Vec::new();
            for lock in &locks {
                let resp = client
                    .release_lock(UnlockRequest {
                        repo: repo.clone(),
                        path: lock.path.clone(),
                        owner: owner.clone(),
                        force,
                    })
                    .await?
                    .into_inner();
                if resp.success {
                    unlocked.push(lock.path.clone());
                } else {
                    let err = if resp.error.is_empty() {
                        "release refused by server".to_string()
                    } else {
                        resp.error
                    };
                    failed.push((lock.path.clone(), err));
                }
            }

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "ok": failed.is_empty(),
                        "unlocked": unlocked,
                        "failed": failed
                            .iter()
                            .map(|(p, e)| json!({"path": p, "error": e}))
                            .collect::<Vec<_>>(),
                    }))?
                );
            } else {
                for p in &unlocked {
                    println!("\x1b[32mUnlocked:\x1b[0m {p}");
                }
                for (p, e) in &failed {
                    eprintln!("\x1b[31mFailed:\x1b[0m {p} — {e}");
                }
                println!(
                    "\nReleased {} lock(s){}",
                    unlocked.len(),
                    if failed.is_empty() {
                        String::new()
                    } else {
                        format!(", {} failed", failed.len())
                    }
                );
            }

            if !failed.is_empty() && !json {
                bail!("{} lock(s) could not be released", failed.len());
            }
            return Ok(());
        }

        // Single-path unlock (existing behaviour).
        let rel_path = path.replace('\\', "/");
        let resp = client
            .release_lock(UnlockRequest {
                repo: repo.clone(),
                path: rel_path.clone(),
                owner: owner.clone(),
                force,
            })
            .await?
            .into_inner();

        if resp.success {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "ok": true,
                        "path": rel_path,
                    }))?
                );
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
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "ok": false,
                        "error": msg,
                        "path": rel_path,
                    }))?
                );
            } else {
                bail!("Failed to unlock '{}': {}", rel_path, msg);
            }
        }

        Ok(())
    })
}
