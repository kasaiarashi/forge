// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! `forge fetch [<branch>]` — download remote branches' objects and update
//! their remote-tracking refs, **without** touching HEAD, the index, or the
//! working tree.
//!
//! Mirrors `git fetch`. The followups are intentionally separate:
//!
//!   * `forge fetch`              → download all remote branches.
//!   * `forge fetch <branch>`     → download just that branch.
//!   * `forge switch <branch>`    → if the local branch is missing, falls
//!                                  through to the remote-tracking ref
//!                                  written here (DWIM, like `git switch`).
//!
//! Stores tips at `.forge/refs/remotes/<remote>/<branch>` so a future
//! `forge branch -a` (and the switch DWIM path) can find them. The
//! workspace's first configured remote is treated as the default — same
//! convention as `default_remote_url()` everywhere else.

use anyhow::{anyhow, bail, Result};
use forge_core::hash::ForgeHash;
use forge_core::workspace::Workspace;
use forge_proto::forge::*;

use crate::commands::pull::fetch_objects_to_tip;

pub fn run(branch: Option<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    let config = ws.config()?;
    let remote = config
        .default_remote()
        .ok_or_else(|| anyhow!("No remote configured. Use: forge remote add origin <url>"))?;
    let remote_name = remote.name.clone();
    let server_url = remote.url.clone();

    let repo_name = if config.repo.is_empty() {
        "default".to_string()
    } else {
        config.repo.clone()
    };

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        fetch_async(&ws, &server_url, &remote_name, &repo_name, branch).await
    })
}

async fn fetch_async(
    ws: &Workspace,
    server_url: &str,
    remote_name: &str,
    repo_name: &str,
    branch_filter: Option<String>,
) -> Result<()> {
    let mut client = crate::client::connect_forge(server_url).await?;

    let refs_resp = client
        .get_refs(GetRefsRequest {
            repo: repo_name.to_string(),
        })
        .await?
        .into_inner();

    // Pull out just the branch refs (refs/heads/...) and strip the prefix
    // so we work in plain branch names from here on.
    let mut remote_branches: Vec<(String, Vec<u8>)> = refs_resp
        .refs
        .into_iter()
        .filter_map(|(name, hash)| {
            name.strip_prefix("refs/heads/")
                .map(|b| (b.to_string(), hash))
        })
        .collect();
    remote_branches.sort_by(|a, b| a.0.cmp(&b.0));

    // Filter to a single branch if requested.
    if let Some(ref want) = branch_filter {
        remote_branches.retain(|(name, _)| name == want);
        if remote_branches.is_empty() {
            bail!("branch '{}' not found on remote", want);
        }
    }

    if remote_branches.is_empty() {
        println!("No branches on remote.");
        return Ok(());
    }

    println!("From {server_url}");

    // For each branch: BFS+fetch, then update the remote-tracking ref.
    // We track each line's status so we can print git-style output (new
    // branch / fast-forward / up-to-date) after the actual work runs.
    for (branch, tip_bytes) in &remote_branches {
        let new_tip = ForgeHash::from_hex(&hex::encode(tip_bytes))?;

        // Diff against any existing remote-tracking ref so we can pick the
        // right status label. Treat "no existing ref" as "new branch", and
        // "matches existing" as "up to date".
        let existing = ws.get_remote_ref(remote_name, branch).ok();

        if existing.as_ref() == Some(&new_tip) {
            println!(
                " = [up to date]      {} -> {}/{}",
                branch, remote_name, branch
            );
            continue;
        }

        // Fetch all objects reachable from this tip. The shared helper
        // handles the resumable-clone case (children of already-on-disk
        // manifests get walked too) so a half-finished previous fetch
        // doesn't leave us with missing chunks.
        let _received =
            fetch_objects_to_tip(ws, &mut client, repo_name, tip_bytes).await?;

        // Update the remote-tracking ref to the new tip.
        ws.set_remote_ref(remote_name, branch, &new_tip)?;

        match existing {
            None => println!(
                " * [new branch]      {} -> {}/{}",
                branch, remote_name, branch
            ),
            Some(old) => println!(
                "   {}..{}  {} -> {}/{}",
                old.short(),
                new_tip.short(),
                branch,
                remote_name,
                branch
            ),
        }
    }

    Ok(())
}
