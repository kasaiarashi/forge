// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{bail, Result};
use forge_core::hash::ForgeHash;
use forge_core::workspace::Workspace;
use forge_proto::forge::forge_service_client::ForgeServiceClient;
use forge_proto::forge::*;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub fn run(force: bool) -> Result<()> {
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
    rt.block_on(async { push_async(&ws, &server_url, &repo_name, force).await })
}

async fn push_async(ws: &Workspace, server_url: &str, repo_name: &str, force: bool) -> Result<()> {
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

    // For force push, collect all objects from local tip (ignore remote tip).
    let stop_hash = if force {
        vec![0u8; 32]
    } else {
        remote_tip_bytes.clone()
    };

    // Collect all objects from local tip back to remote tip (or all if force).
    let mut seen = HashSet::new();
    let mut objects_to_push = Vec::new();
    collect_snapshot_objects(ws, &local_tip, &stop_hash, &mut objects_to_push, &mut seen)?;

    if objects_to_push.is_empty() && !force {
        println!("Everything up to date.");
        return Ok(());
    }

    // Check which objects the server already has.
    let missing = if objects_to_push.is_empty() {
        vec![]
    } else {
        let hashes: Vec<Vec<u8>> = objects_to_push.iter().map(|h| h.as_bytes().to_vec()).collect();
        let has_resp = client
            .has_objects(HasObjectsRequest {
                hashes: hashes.clone(),
                repo: repo_name.to_string(),
            })
            .await?
            .into_inner();

        objects_to_push
            .iter()
            .zip(has_resp.has.iter())
            .filter(|(_, &has)| !has)
            .map(|(h, _)| *h)
            .collect::<Vec<ForgeHash>>()
    };

    if !missing.is_empty() {
        // Calculate total bytes to push.
        let store = forge_core::store::chunk_store::ChunkStore::new(ws.forge_dir().join("objects"));
        let mut total_bytes: u64 = 0;
        let mut object_sizes: Vec<(ForgeHash, u64)> = Vec::with_capacity(missing.len());
        for hash in &missing {
            match store.get(hash) {
                Ok(data) => {
                    let size = data.len() as u64;
                    total_bytes += size;
                    object_sizes.push((*hash, size));
                }
                Err(_) => continue,
            }
        }

        let obj_count = object_sizes.len();
        println!(
            "Pushing {} object(s), {:.2} MiB total",
            obj_count,
            total_bytes as f64 / (1024.0 * 1024.0)
        );

        // Set up progress bar.
        let pb = ProgressBar::new(total_bytes);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{msg}\n{wide_bar:.cyan/blue} {percent}% ({bytes}/{total_bytes}) {bytes_per_sec} ETA {eta}")
                .expect("valid template")
                .progress_chars("=>-"),
        );
        pb.set_message(format!("Writing objects: {obj_count} objects"));

        let bytes_sent = Arc::new(AtomicU64::new(0));
        let objects_sent = Arc::new(AtomicU64::new(0));

        // Stream objects to server.
        let (tx, rx) = tokio::sync::mpsc::channel(32);

        let forge_dir = ws.forge_dir();
        let repo_name_clone = repo_name.to_string();
        let bytes_sent_clone = Arc::clone(&bytes_sent);
        let objects_sent_clone = Arc::clone(&objects_sent);
        let obj_count_u64 = obj_count as u64;

        tokio::spawn(async move {
            let store = forge_core::store::chunk_store::ChunkStore::new(forge_dir.join("objects"));
            for (hash, _) in object_sizes {
                match store.get(&hash) {
                    Ok(data) => {
                        let data_len = data.len() as u64;
                        let chunk = ObjectChunk {
                            hash: hash.as_bytes().to_vec(),
                            object_type: 0,
                            total_size: data_len,
                            offset: 0,
                            data,
                            is_last: true,
                            repo: repo_name_clone.clone(),
                        };
                        if tx.send(chunk).await.is_err() {
                            break;
                        }
                        bytes_sent_clone.fetch_add(data_len, Ordering::Relaxed);
                        objects_sent_clone.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => continue,
                }
            }
        });

        // Tick progress bar while streaming.
        let pb_clone = pb.clone();
        let bytes_sent_tick = Arc::clone(&bytes_sent);
        let objects_sent_tick = Arc::clone(&objects_sent);
        let tick_handle = tokio::spawn(async move {
            loop {
                let sent = bytes_sent_tick.load(Ordering::Relaxed);
                let obj_done = objects_sent_tick.load(Ordering::Relaxed);
                pb_clone.set_position(sent);
                pb_clone.set_message(format!("Writing objects: {obj_done}/{obj_count_u64} objects"));
                if sent >= total_bytes {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        client.push_objects(stream).await?;

        // Ensure progress bar completes.
        pb.set_position(total_bytes);
        pb.set_message(format!("Writing objects: {obj_count}/{obj_count} objects"));
        pb.finish_with_message(format!("Writing objects: {obj_count}/{obj_count} objects, done."));
        tick_handle.abort();
    }

    // Update remote ref.
    // For force push, send the current remote tip as old_hash (valid CAS),
    // or zero if the ref doesn't exist yet.
    let old_hash_for_cas = if force {
        remote_tip_bytes.clone()
    } else {
        remote_tip_bytes
    };

    let update_resp = client
        .update_ref(UpdateRefRequest {
            repo: repo_name.to_string(),
            ref_name: ref_name.clone(),
            old_hash: old_hash_for_cas,
            new_hash: local_tip.as_bytes().to_vec(),
        })
        .await?
        .into_inner();

    if update_resp.success {
        if force {
            println!("Pushed to {} -> {} (forced)", ref_name, local_tip.short());
        } else {
            println!("Pushed to {} -> {}", ref_name, local_tip.short());
        }
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
    seen: &mut HashSet<ForgeHash>,
) -> Result<()> {
    if tip.is_zero() || tip.as_bytes().as_slice() == stop_hash || !seen.insert(*tip) {
        return Ok(());
    }

    objects.push(*tip);

    let snapshot = ws.object_store.get_snapshot(tip)?;
    collect_tree_objects(ws, &snapshot.tree, objects, seen)?;

    for parent in &snapshot.parents {
        collect_snapshot_objects(ws, parent, stop_hash, objects, seen)?;
    }

    Ok(())
}

fn collect_tree_objects(
    ws: &Workspace,
    tree_hash: &ForgeHash,
    objects: &mut Vec<ForgeHash>,
    seen: &mut HashSet<ForgeHash>,
) -> Result<()> {
    if !seen.insert(*tree_hash) {
        return Ok(());
    }
    objects.push(*tree_hash);

    let tree = ws.object_store.get_tree(tree_hash)?;
    for entry in &tree.entries {
        match entry.kind {
            forge_core::object::tree::EntryKind::Directory => {
                collect_tree_objects(ws, &entry.hash, objects, seen)?;
            }
            _ => {
                if seen.insert(entry.hash) {
                    objects.push(entry.hash);
                    if let Ok(chunked) = ws.object_store.get_chunked_blob(&entry.hash) {
                        for chunk_ref in &chunked.chunks {
                            if seen.insert(chunk_ref.hash) {
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
