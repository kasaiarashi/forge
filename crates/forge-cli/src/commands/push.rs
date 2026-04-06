// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{bail, Result};
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
    rt.block_on(async { push_async(&ws, &server_url, &repo_name).await })
}

async fn push_async(ws: &Workspace, server_url: &str, repo_name: &str) -> Result<()> {
    let mut client = ForgeServiceClient::connect(server_url.to_string()).await?;

    // Get current branch and its tip.
    let branch = ws
        .current_branch()?
        .ok_or_else(|| anyhow::anyhow!("HEAD is detached; switch to a branch first"))?;
    let local_tip = ws.get_branch_tip(&branch)?;
    let ref_name = format!("refs/heads/{}", branch);

    if local_tip.is_zero() {
        println!("Nothing to push (no snapshots).");
        return Ok(());
    }

    // Get remote ref.
    let refs_resp = client
        .get_refs(GetRefsRequest {
            repo: repo_name.to_string(),
        })
        .await?
        .into_inner();

    let remote_tip_bytes = refs_resp.refs.get(&ref_name).cloned().unwrap_or_else(|| vec![0u8; 32]);

    // Collect all objects from local tip back to remote tip.
    let mut objects_to_push = Vec::new();
    collect_snapshot_objects(ws, &local_tip, &remote_tip_bytes, &mut objects_to_push)?;

    if objects_to_push.is_empty() {
        println!("Everything up to date.");
        return Ok(());
    }

    // Check which objects the server already has.
    let hashes: Vec<Vec<u8>> = objects_to_push.iter().map(|h| h.as_bytes().to_vec()).collect();
    let has_resp = client
        .has_objects(HasObjectsRequest {
            hashes: hashes.clone(),
            repo: repo_name.to_string(),
        })
        .await?
        .into_inner();

    let missing: Vec<ForgeHash> = objects_to_push
        .iter()
        .zip(has_resp.has.iter())
        .filter(|(_, &has)| !has)
        .map(|(h, _)| *h)
        .collect();

    if missing.is_empty() {
        // Server has all objects, just update the ref.
    } else {
        println!("Pushing {} object(s)...", missing.len());

        // Stream objects to server.
        let (tx, rx) = tokio::sync::mpsc::channel(32);

        let forge_dir = ws.forge_dir();
        let missing_clone = missing.clone();
        let repo_name_clone = repo_name.to_string();

        tokio::spawn(async move {
            let store = forge_core::store::chunk_store::ChunkStore::new(forge_dir.join("objects"));
            for hash in missing_clone {
                match store.get(&hash) {
                    Ok(data) => {
                        let chunk = ObjectChunk {
                            hash: hash.as_bytes().to_vec(),
                            object_type: 0,
                            total_size: data.len() as u64,
                            offset: 0,
                            data,
                            is_last: true,
                            repo: repo_name_clone.clone(),
                        };
                        if tx.send(chunk).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => continue,
                }
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        client.push_objects(stream).await?;
    }

    // Update remote ref (CAS).
    let update_resp = client
        .update_ref(UpdateRefRequest {
            repo: repo_name.to_string(),
            ref_name: ref_name.clone(),
            old_hash: remote_tip_bytes,
            new_hash: local_tip.as_bytes().to_vec(),
        })
        .await?
        .into_inner();

    if update_resp.success {
        println!("Pushed to {} -> {}", ref_name, local_tip.short());
    } else {
        bail!(
            "Failed to update ref: {}. Someone else may have pushed.",
            update_resp.error
        );
    }

    Ok(())
}

/// Walk the snapshot chain from `tip` and collect all reachable object hashes,
/// stopping when we reach an object whose hash matches `stop_hash`.
fn collect_snapshot_objects(
    ws: &Workspace,
    tip: &ForgeHash,
    stop_hash: &[u8],
    objects: &mut Vec<ForgeHash>,
) -> Result<()> {
    if tip.is_zero() || tip.as_bytes().as_slice() == stop_hash {
        return Ok(());
    }

    // Add the snapshot object itself.
    objects.push(*tip);

    let snapshot = ws.object_store.get_snapshot(tip)?;

    // Add tree objects recursively.
    collect_tree_objects(ws, &snapshot.tree, objects)?;

    // Recurse into parents (stop at remote tip).
    for parent in &snapshot.parents {
        collect_snapshot_objects(ws, parent, stop_hash, objects)?;
    }

    Ok(())
}

fn collect_tree_objects(
    ws: &Workspace,
    tree_hash: &ForgeHash,
    objects: &mut Vec<ForgeHash>,
) -> Result<()> {
    if objects.contains(tree_hash) {
        return Ok(());
    }
    objects.push(*tree_hash);

    let tree = ws.object_store.get_tree(tree_hash)?;
    for entry in &tree.entries {
        match entry.kind {
            forge_core::object::tree::EntryKind::Directory => {
                collect_tree_objects(ws, &entry.hash, objects)?;
            }
            _ => {
                if !objects.contains(&entry.hash) {
                    objects.push(entry.hash);
                    // For chunked blobs, also collect the chunk data objects.
                    if let Ok(chunked) = ws.object_store.get_chunked_blob(&entry.hash) {
                        for chunk_ref in &chunked.chunks {
                            if !objects.contains(&chunk_ref.hash) {
                                objects.push(chunk_ref.hash);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
