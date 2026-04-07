// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::Result;
use forge_core::hash::ForgeHash;
use forge_core::workspace::Workspace;
use forge_proto::forge::forge_service_client::ForgeServiceClient;
use forge_proto::forge::*;

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let config = ws.config()?;

    let server_url = config
        .default_remote_url()
        .ok_or_else(|| anyhow::anyhow!("No remote configured. Use: forge remote add origin <url>"))?
        .to_string();

    let repo_name = if config.repo.is_empty() {
        "default".to_string()
    } else {
        config.repo.clone()
    };

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async { pull_async(&ws, &server_url, &repo_name).await })
}

async fn pull_async(ws: &Workspace, server_url: &str, repo_name: &str) -> Result<()> {
    let mut client = ForgeServiceClient::connect(server_url.to_string()).await?;

    let branch = ws
        .current_branch()?
        .ok_or_else(|| anyhow::anyhow!("HEAD is detached"))?;
    let ref_name = format!("refs/heads/{}", branch);

    // Get remote refs.
    let refs_resp = client
        .get_refs(GetRefsRequest {
            repo: repo_name.to_string(),
        })
        .await?
        .into_inner();

    let remote_tip_bytes = match refs_resp.refs.get(&ref_name) {
        Some(h) => h.clone(),
        None => {
            println!("Branch '{}' does not exist on remote.", branch);
            return Ok(());
        }
    };

    let remote_tip = ForgeHash::from_hex(&hex::encode(&remote_tip_bytes))?;
    let local_tip = ws.get_branch_tip(&branch)?;

    if remote_tip == local_tip {
        println!("Already up to date.");
        return Ok(());
    }

    println!("Pulling from remote...");

    // Request all objects from the remote tip that we don't have locally.
    // Start by requesting the snapshot object and work from there.
    let mut want = vec![remote_tip_bytes.clone()];
    let mut received = 0u64;

    // Iteratively pull objects we're missing.
    loop {
        if want.is_empty() {
            break;
        }

        // Filter to objects we don't already have.
        let need: Vec<Vec<u8>> = want
            .iter()
            .filter(|h| {
                let hex = hex::encode(h);
                ForgeHash::from_hex(&hex)
                    .map(|fh| !ws.object_store.has(&fh))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        if need.is_empty() {
            break;
        }

        let mut stream = client
            .pull_objects(PullRequest {
                want_hashes: need,
                repo: repo_name.to_string(),
            })
            .await?
            .into_inner();

        want.clear();

        let mut current_data = Vec::new();
        let mut current_hash: Option<Vec<u8>> = None;

        while let Some(chunk) = stream.message().await? {
            if current_hash.as_ref() != Some(&chunk.hash) {
                current_data.clear();
                current_hash = Some(chunk.hash.clone());
            }

            current_data.extend_from_slice(&chunk.data);

            if chunk.is_last {
                let hash_hex = hex::encode(&chunk.hash);
                let forge_hash = ForgeHash::from_hex(&hash_hex)?;

                // Verify received data matches claimed hash.
                let computed = ForgeHash::from_bytes(&current_data);
                if computed != forge_hash {
                    anyhow::bail!(
                        "integrity error: server sent corrupt object (claimed {}, got {})",
                        forge_hash.short(),
                        computed.short()
                    );
                }

                ws.object_store
                    .chunks
                    .put(&forge_hash, &current_data)?;

                received += 1;

                // Try to parse as snapshot to discover more objects to pull.
                if let Ok(snapshot) = ws.object_store.get_snapshot(&forge_hash) {
                    want.push(snapshot.tree.as_bytes().to_vec());
                    for parent in &snapshot.parents {
                        if !parent.is_zero() {
                            want.push(parent.as_bytes().to_vec());
                        }
                    }
                }

                // Try to parse as tree to discover blob objects.
                if let Ok(tree) = ws.object_store.get_tree(&forge_hash) {
                    for entry in &tree.entries {
                        want.push(entry.hash.as_bytes().to_vec());
                    }
                }

                // Try to parse as chunked blob to discover chunk objects.
                if let Ok(chunked) = ws.object_store.get_chunked_blob(&forge_hash) {
                    for chunk_ref in &chunked.chunks {
                        want.push(chunk_ref.hash.as_bytes().to_vec());
                    }
                }

                current_data.clear();
                current_hash = None;
            }
        }
    }

    // Fast-forward local branch.
    let old_tip = local_tip.short();
    ws.set_branch_tip(&branch, &remote_tip)?;
    println!("Receiving objects: {} done.", received);
    println!("   {}..{} {} -> {}", old_tip, remote_tip.short(), branch, branch);

    Ok(())
}
