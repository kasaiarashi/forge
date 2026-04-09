// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{bail, Result};
use forge_core::hash::ForgeHash;
use forge_core::workspace::Workspace;
use forge_proto::forge::*;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashSet;

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

    let remote_name = config
        .default_remote()
        .map(|r| r.name.clone())
        .unwrap_or_else(|| "origin".to_string());

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async { push_async(&ws, &server_url, &repo_name, &remote_name, force).await })
}

async fn push_async(ws: &Workspace, server_url: &str, repo_name: &str, remote_name: &str, force: bool) -> Result<()> {
    let mut client = crate::client::connect_forge(server_url).await?;

    // Get current branch and its tip.
    let branch = match ws.current_branch()? {
        Some(b) => b,
        None => {
            // Detached HEAD. Build a recovery hint: if new commits have
            // been made in this state, the user needs to promote them to
            // a branch or they'll eventually be garbage-collected.
            let head = ws.head_snapshot()?;
            let mut msg = String::from(
                "HEAD is detached; push needs a branch to target.\n",
            );
            if !head.is_zero() {
                msg.push_str(&format!(
                    "\nCurrent commit: {}\n",
                    head.short()
                ));
                msg.push_str(
                    "\nTo save any commits you made in detached mode and push them:\n\
                     \n    forge branch <new-name>     # create a branch at this commit\n\
                     \x20   forge switch <new-name>\n\
                     \x20   forge push\n\
                     \n\
                     If you just want to go back to an existing branch (discarding \
                     anything committed in detached mode), run `forge switch <branch>`.",
                );
            } else {
                msg.push_str(
                    "\nSwitch to a branch first:  forge switch <branch>",
                );
            }
            bail!("{msg}");
        }
    };
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

    let stop_hash = if force {
        vec![0u8; 32]
    } else {
        remote_tip_bytes.clone()
    };

    // Collect all objects from local tip back to remote tip.
    let mut seen = HashSet::new();
    let mut objects_to_push = Vec::new();
    collect_snapshot_objects(ws, &local_tip, &stop_hash, &mut objects_to_push, &mut seen)?;

    if objects_to_push.is_empty() && !force {
        println!("Everything up to date.");
        return Ok(());
    }

    // Check which objects the server already has.
    // For small pushes (<100 objects), skip the has_objects round-trip — server deduplicates via put_raw.
    let missing = if objects_to_push.is_empty() {
        vec![]
    } else if objects_to_push.len() < 100 {
        // Small push: skip has_objects check, just push everything (server deduplicates).
        objects_to_push.clone()
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
        let store = forge_core::store::chunk_store::ChunkStore::new(ws.forge_dir().join("objects"));

        // Read objects in parallel from disk.
        let raw_objects: Vec<(ForgeHash, Vec<u8>)> = {
            use rayon::prelude::*;
            missing
                .par_iter()
                .filter_map(|hash| store.get_raw(hash).ok().map(|data| (*hash, data)))
                .collect()
        };
        let total_bytes: u64 = raw_objects.iter().map(|(_, d)| d.len() as u64).sum();

        let obj_count = raw_objects.len();
        let show_progress = total_bytes > 1024 * 1024; // progress bar for >1 MiB

        if show_progress {
            println!(
                "Pushing {} object(s), {:.2} MiB total",
                obj_count,
                total_bytes as f64 / (1024.0 * 1024.0)
            );
        }

        let pb = if show_progress {
            let pb = ProgressBar::new(total_bytes);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{msg}\n{wide_bar:.cyan/blue} {percent}% ({bytes}/{total_bytes}) {bytes_per_sec} ETA {eta}")
                    .expect("valid template")
                    .progress_chars("=>-"),
            );
            pb.set_message(format!("Writing objects: 0/{obj_count} objects"));
            pb
        } else {
            ProgressBar::hidden()
        };

        let repo_name_owned = repo_name.to_string();
        let chunks: Vec<ObjectChunk> = raw_objects
            .into_iter()
            .map(|(hash, compressed_data)| ObjectChunk {
                hash: hash.as_bytes().to_vec(),
                object_type: 1, // 1 = pre-compressed
                total_size: compressed_data.len() as u64,
                offset: 0,
                data: compressed_data,
                is_last: true,
                repo: repo_name_owned.clone(),
            })
            .collect();

        let mut sent_bytes: u64 = 0;
        let mut sent_objs: usize = 0;
        let (tx, rx) = tokio::sync::mpsc::channel::<ObjectChunk>(64);

        let pb_clone = pb.clone();
        let send_handle = tokio::spawn(async move {
            for chunk in chunks {
                let data_len = chunk.total_size;
                if tx.send(chunk).await.is_err() {
                    break;
                }
                sent_bytes += data_len;
                sent_objs += 1;
                pb_clone.set_position(sent_bytes);
                pb_clone.set_message(format!("Writing objects: {sent_objs}/{obj_count} objects"));
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        client.push_objects(stream).await?;
        send_handle.abort();

        pb.set_position(total_bytes);
        if show_progress {
            pb.finish_with_message(format!("Writing objects: {obj_count}/{obj_count} objects, done."));
        }
    }

    // Update remote ref. Pass `force` through so the server knows whether
    // to do an atomic compare-and-swap (default) or an unconditional
    // overwrite (--force).
    let update_resp = client
        .update_ref(UpdateRefRequest {
            repo: repo_name.to_string(),
            ref_name: ref_name.clone(),
            old_hash: remote_tip_bytes.clone(),
            new_hash: local_tip.as_bytes().to_vec(),
            force,
        })
        .await?
        .into_inner();

    if update_resp.success {
        let remote_short = ForgeHash::from_hex(&hex::encode(&remote_tip_bytes))
            .map(|h| h.short())
            .unwrap_or_else(|_| "(new)".to_string());
        if force {
            println!(
                " + {}...{} {} -> {}/{} (forced)",
                remote_short, local_tip.short(), branch, remote_name, branch
            );
        } else {
            println!(
                "   {}..{} {} -> {}/{}",
                remote_short, local_tip.short(), branch, remote_name, branch
            );
        }
    } else if force {
        bail!(
            "Force push to {} rejected: {}.",
            ref_name,
            update_resp.error
        );
    } else {
        bail!(
            "Push to {} rejected: {}.\n\
             Pull and rebase, then push again — or use `forge push --force` \
             to overwrite the remote (rewrites history; use with care).",
            ref_name,
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
