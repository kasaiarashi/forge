// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{bail, Result};
use forge_core::hash::ForgeHash;
use forge_core::workspace::Workspace;
use forge_proto::forge::*;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashSet;

pub fn run(force: bool, remote_arg: Option<&str>, branch_arg: Option<&str>) -> Result<()> {
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

    // Git-compat positional args. Forge only supports a single configured
    // remote and always pushes the current branch, so we accept `forge push
    // origin main` but reject any value that doesn't match — silently
    // ignoring a mismatch would push to a different target than the user
    // asked for.
    if let Some(r) = remote_arg {
        if r != remote_name {
            bail!(
                "remote '{}' is not configured (current: '{}'). Use `forge remote` to inspect.",
                r,
                remote_name
            );
        }
    }
    if let Some(b) = branch_arg {
        if let Some(current) = ws.current_branch()? {
            if b != current {
                bail!(
                    "forge push always targets the current branch ('{}'). Switch first: forge switch {}",
                    current,
                    b
                );
            }
        }
    }

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
    let remote_is_zero = remote_tip_bytes.iter().all(|&b| b == 0);

    // Pre-flight fast-forward check: make sure remote_tip is an ancestor of
    // local_tip before uploading anything. Server enforces this too, but
    // checking here avoids a wasted object upload and gives a clearer error.
    if !force && !remote_is_zero {
        let remote_tip = ForgeHash::from_hex(&hex::encode(&remote_tip_bytes))?;
        if !is_local_ancestor_of(ws, &remote_tip, &local_tip)? {
            bail!(
                "Updates were rejected because the remote contains work that you do \
                 not have locally.\n\
                 This is usually caused by another repository pushing to the same ref.\n\
                 Pull and rebase, then push again — or use `forge push --force` \
                 to overwrite the remote (rewrites history; use with care)."
            );
        }
    }

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

    // Allocate a session id for this push. The same id is sent on every
    // chunk of PushObjects and again in CommitPush; it lets the server
    // stage objects per-session and make the final promote + ref CAS
    // atomic. UUIDv7 embeds a timestamp so sweeper logs sort nicely.
    let session_id = uuid::Uuid::now_v7().to_string();

    if !missing.is_empty() {
        let store = forge_core::store::chunk_store::ChunkStore::new(ws.forge_dir().join("objects"));

        // Stat files to get total size without reading them all into memory.
        let obj_count = missing.len();
        let total_bytes: u64 = missing
            .iter()
            .filter_map(|h| store.file_size(h))
            .sum();
        let show_progress = total_bytes > 1024 * 1024;

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

        // Two-stage pipeline on dedicated OS threads:
        //   Stage 1: rayon reads objects from disk in parallel → crossbeam channel
        //   Stage 2: single thread chunks and sends to tokio channel (preserves ordering)
        // This keeps blocking I/O off the tokio runtime while ensuring
        // multi-chunk objects are never interleaved in the stream.
        const CHUNK_SIZE: usize = 4 * 1024 * 1024;
        let repo_name_owned = repo_name.to_string();
        let session_id_owned = session_id.clone();
        // gRPC channel holds ≤4 MiB chunks, so 64 slots = 256 MiB max.
        // Read channel holds full objects — keep small for large assets.
        let (grpc_tx, rx) = tokio::sync::mpsc::channel::<ObjectChunk>(64);
        let (read_tx, read_rx) = crossbeam_channel::bounded::<(ForgeHash, Vec<u8>)>(8);

        // Stage 1: parallel disk reads.
        let reader_handle = std::thread::spawn(move || {
            use rayon::prelude::*;
            missing.par_iter().for_each(|hash| {
                if let Ok(data) = store.get_raw(hash) {
                    let _ = read_tx.send((*hash, data));
                }
            });
        });

        // Stage 2: single thread does chunking + sends to gRPC stream.
        let pb_clone = pb.clone();
        let sender_handle = std::thread::spawn(move || {
            let mut sent_bytes: u64 = 0;
            let mut sent_objs: usize = 0;
            while let Ok((hash, data)) = read_rx.recv() {
                let hash_bytes = hash.as_bytes().to_vec();
                if data.len() <= CHUNK_SIZE {
                    let data_len = data.len() as u64;
                    if grpc_tx.blocking_send(ObjectChunk {
                        hash: hash_bytes,
                        object_type: 1,
                        total_size: data_len,
                        offset: 0,
                        data,
                        is_last: true,
                        repo: repo_name_owned.clone(),
                        upload_session_id: session_id_owned.clone(),
                    }).is_err() {
                        break;
                    }
                    sent_bytes += data_len;
                } else {
                    let total = data.len() as u64;
                    for (i, slice) in data.chunks(CHUNK_SIZE).enumerate() {
                        let off = (i * CHUNK_SIZE) as u64;
                        let is_last = off + slice.len() as u64 == total;
                        let slice_len = slice.len() as u64;
                        if grpc_tx.blocking_send(ObjectChunk {
                            hash: hash_bytes.clone(),
                            object_type: 1,
                            total_size: total,
                            offset: off,
                            data: slice.to_vec(),
                            is_last,
                            repo: repo_name_owned.clone(),
                            upload_session_id: session_id_owned.clone(),
                        }).is_err() {
                            break;
                        }
                        sent_bytes += slice_len;
                    }
                }
                sent_objs += 1;
                pb_clone.set_position(sent_bytes);
                pb_clone.set_message(format!("Writing objects: {sent_objs}/{obj_count} objects"));
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        client.push_objects(stream).await?;
        let _ = reader_handle.join();
        let _ = sender_handle.join();

        pb.set_position(total_bytes);
        if show_progress {
            pb.finish_with_message(format!("Writing objects: {obj_count}/{obj_count} objects, done."));
        }
    }

    // Finalise the push: promote staged objects into the live tree and
    // apply the ref update atomically. The server keeps CommitPush
    // idempotent for the same `upload_session_id`, so we can retry on a
    // transient error without worrying about double-applying.
    let ref_update = RefUpdate {
        ref_name: ref_name.clone(),
        old_hash: remote_tip_bytes.clone(),
        new_hash: local_tip.as_bytes().to_vec(),
        force,
    };
    let commit_req = CommitPushRequest {
        repo: repo_name.to_string(),
        upload_session_id: session_id.clone(),
        ref_updates: vec![ref_update],
        touched_paths: Vec::new(),
    };

    // One automatic retry on a transient network error. The session id is
    // the same across both attempts, so if the server committed on the
    // first call the retry hits the idempotent-replay path.
    let commit_resp = match client.commit_push(commit_req.clone()).await {
        Ok(r) => r.into_inner(),
        Err(e)
            if matches!(
                e.code(),
                tonic::Code::Unavailable
                    | tonic::Code::DeadlineExceeded
                    | tonic::Code::Aborted
            ) =>
        {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            client.commit_push(commit_req).await?.into_inner()
        }
        Err(e) => return Err(e.into()),
    };

    if commit_resp.success {
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
    } else if !commit_resp.blocking_locks.is_empty() {
        let mut msg = String::from(
            "Push rejected: one or more paths are locked by another user.\n",
        );
        for lock in &commit_resp.blocking_locks {
            msg.push_str(&format!(
                "    {}  (locked by {})\n",
                lock.path, lock.owner
            ));
        }
        msg.push_str(
            "\nAsk the lock holder to release, or use `forge locks` to \
             inspect. Non-blocking workflow:  `forge pull`, coordinate \
             with the owner, then push again.",
        );
        bail!("{msg}");
    } else if force {
        bail!(
            "Force push to {} rejected: {}.",
            ref_name,
            commit_resp.error
        );
    } else {
        bail!(
            "Push to {} rejected: {}.\n\
             Pull and rebase, then push again — or use `forge push --force` \
             to overwrite the remote (rewrites history; use with care).",
            ref_name,
            commit_resp.error
        );
    }

    Ok(())
}


/// Return true if `ancestor` is reachable from `descendant` via parent links
/// (or equal). Used as a client-side fast-forward pre-flight.
fn is_local_ancestor_of(
    ws: &Workspace,
    ancestor: &ForgeHash,
    descendant: &ForgeHash,
) -> Result<bool> {
    if ancestor == descendant || ancestor.is_zero() {
        return Ok(true);
    }
    let mut seen = HashSet::new();
    let mut stack = vec![*descendant];
    while let Some(cur) = stack.pop() {
        if cur.is_zero() || !seen.insert(cur) {
            continue;
        }
        let snap = match ws.object_store.get_snapshot(&cur) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for p in &snap.parents {
            if p == ancestor {
                return Ok(true);
            }
            stack.push(*p);
        }
    }
    Ok(false)
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
