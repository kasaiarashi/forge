// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

use anyhow::{bail, Result};
use forge_core::hash::ForgeHash;
use forge_core::workspace::Workspace;
use forge_proto::forge::*;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Sidecar file that survives across `forge push` invocations so an
/// interrupted push can resume on the server instead of re-uploading
/// the entire object set. Matches the server-side upload session's
/// 1h TTL; older sidecars are discarded.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSession {
    session_id: String,
    repo: String,
    ref_name: String,
    local_tip: String,
    remote_tip: String,
    created_at: i64,
}

/// Seconds of wall time after which a persisted session file is
/// assumed stale. Matches the server default upload TTL. We don't
/// trust the client clock alone — `QueryUploadSession` is the
/// authoritative check before any real work happens.
const SESSION_FILE_TTL_SECS: i64 = 60 * 60;

fn session_file_path(ws: &Workspace) -> PathBuf {
    ws.forge_dir().join("last_push_session.json")
}

fn load_session_if_fresh(
    ws: &Workspace,
    repo: &str,
    ref_name: &str,
    local_tip_hex: &str,
    remote_tip_hex: &str,
) -> Option<PersistedSession> {
    let path = session_file_path(ws);
    let raw = std::fs::read_to_string(&path).ok()?;
    let parsed: PersistedSession = serde_json::from_str(&raw).ok()?;
    // Only reuse if every dimension matches — a local commit or a
    // remote rebase between attempts means the object set has shifted.
    if parsed.repo != repo
        || parsed.ref_name != ref_name
        || parsed.local_tip != local_tip_hex
        || parsed.remote_tip != remote_tip_hex
    {
        return None;
    }
    let now = chrono::Utc::now().timestamp();
    if now.saturating_sub(parsed.created_at) > SESSION_FILE_TTL_SECS {
        return None;
    }
    Some(parsed)
}

fn save_session(ws: &Workspace, session: &PersistedSession) {
    let path = session_file_path(ws);
    if let Ok(json) = serde_json::to_string_pretty(session) {
        let _ = std::fs::write(&path, json);
    }
}

fn clear_session(ws: &Workspace) {
    let _ = std::fs::remove_file(session_file_path(ws));
}

pub fn run(force: bool, remote_arg: Option<&str>, branch_arg: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    run_in(&cwd, force, remote_arg, branch_arg)
}

/// Variant that takes an explicit starting directory so the Phase-4
/// FFI layer can push without mutating the process-wide CWD.
pub fn run_in(
    cwd: &std::path::Path,
    force: bool,
    remote_arg: Option<&str>,
    branch_arg: Option<&str>,
) -> Result<()> {
    let ws = Workspace::discover(cwd)?;
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
    let mut client = crate::client::connect_forge_write(server_url).await?;

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
    let mut missing: Vec<ForgeHash> = if objects_to_push.is_empty() {
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

    // Session allocation: reuse a persisted session from a prior
    // interrupted push when the workspace state lines up (same repo,
    // ref, local + remote tips) AND the server still has it, else
    // mint a fresh UUIDv7. UUIDv7 embeds a timestamp so sweeper logs
    // sort nicely.
    let local_tip_hex = hex::encode(local_tip.as_bytes());
    let remote_tip_hex = hex::encode(&remote_tip_bytes);

    let mut resumed_progress: Option<HashMap<Vec<u8>, (u64, u64)>> = None;
    let session_id = match load_session_if_fresh(
        ws,
        repo_name,
        &ref_name,
        &local_tip_hex,
        &remote_tip_hex,
    ) {
        Some(persisted) => {
            // Probe the server — the session may have been swept even
            // if our local sidecar is still inside its TTL.
            let q = client
                .query_upload_session(QueryUploadSessionRequest {
                    repo: repo_name.to_string(),
                    upload_session_id: persisted.session_id.clone(),
                })
                .await;
            match q {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    match resp.state.as_str() {
                        "uploading" => {
                            let map: HashMap<Vec<u8>, (u64, u64)> = resp
                                .objects
                                .iter()
                                .map(|o| {
                                    (o.hash.clone(), (o.received_bytes, o.declared_size))
                                })
                                .collect();
                            if !map.is_empty() {
                                println!(
                                    "Resuming push (session {}, {} object(s) already known to server)",
                                    &persisted.session_id[..persisted.session_id.len().min(8)],
                                    map.len(),
                                );
                            }
                            resumed_progress = Some(map);
                            persisted.session_id
                        }
                        "committed" => {
                            // Server already committed. Short-circuit
                            // to the CommitPush path so the idempotent
                            // replay surfaces the original result.
                            println!("Server already committed this push — replaying result.");
                            persisted.session_id
                        }
                        "failed" | "abandoned" => {
                            // Prior attempt terminally failed on the
                            // server. Start fresh so the user doesn't
                            // get stuck with an unrecoverable session.
                            clear_session(ws);
                            uuid::Uuid::now_v7().to_string()
                        }
                        _ => {
                            // Empty state = unknown session (TTL).
                            // Other = unexpected string — reset.
                            clear_session(ws);
                            uuid::Uuid::now_v7().to_string()
                        }
                    }
                }
                Err(_) => {
                    // Treat query failure as "fresh push"; the worst
                    // case is re-uploading the objects the server
                    // already has, which is dedup'd server-side.
                    clear_session(ws);
                    uuid::Uuid::now_v7().to_string()
                }
            }
        }
        None => uuid::Uuid::now_v7().to_string(),
    };

    // Persist session early so a mid-push crash leaves a resume hint
    // on disk. Expectation: `clear_session` runs on successful commit.
    save_session(
        ws,
        &PersistedSession {
            session_id: session_id.clone(),
            repo: repo_name.to_string(),
            ref_name: ref_name.clone(),
            local_tip: local_tip_hex.clone(),
            remote_tip: remote_tip_hex.clone(),
            created_at: chrono::Utc::now().timestamp(),
        },
    );

    // Filter `missing` against the resume snapshot so fully-received
    // objects are dropped from the upload list. Partial objects stay
    // — the current streaming layer can't resume mid-object yet
    // (Phase 3e packfile work will revisit), so we re-upload them
    // and the server dedups via content addressing.
    if let Some(ref progress) = resumed_progress {
        missing.retain(|h| {
            let hash_bytes = h.as_bytes().to_vec();
            match progress.get(&hash_bytes) {
                Some(&(received, declared)) => {
                    declared == 0 || received < declared
                }
                None => true,
            }
        });
    }

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
        // Read channel holds full objects — bumped 8→256 so the rayon
        // disk-read workers don't park on every transient gRPC
        // backpressure. At 17 KiB avg object size (typical UE push)
        // this buys ~4 MiB of pipeline buffer, which is ~33 ms at a
        // sustained 100 MB/s — enough to absorb a few WINDOW_UPDATE
        // stalls without draining rayon. Worst-case memory is bounded
        // by `max_object_size` (16 GiB default) × 256 which is absurd
        // in practice but cheap to bound tighter if a real deployment
        // ever hits it.
        let (grpc_tx, rx) = tokio::sync::mpsc::channel::<ObjectChunk>(64);
        let (read_tx, read_rx) = crossbeam_channel::bounded::<(ForgeHash, Vec<u8>)>(256);

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
        // Progress-bar updates are throttled — `set_position` fires on
        // every chunk (indicatif debounces its own redraw cheaply) but
        // `set_message` runs `format!` + interior-mutability state
        // updates, which at 10k-100k objects/sec becomes a non-trivial
        // fraction of the sender's CPU budget. Rate-limit to every
        // 512 objects OR every 100 ms, whichever comes first.
        let pb_clone = pb.clone();
        let sender_handle = std::thread::spawn(move || {
            use std::time::{Duration, Instant};
            let mut sent_bytes: u64 = 0;
            let mut sent_objs: usize = 0;
            let mut last_msg_objs: usize = 0;
            let mut last_msg_at = Instant::now();
            let msg_min_interval = Duration::from_millis(100);
            let msg_min_objects: usize = 512;
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
                if sent_objs - last_msg_objs >= msg_min_objects
                    || last_msg_at.elapsed() >= msg_min_interval
                {
                    pb_clone.set_message(format!(
                        "Writing objects: {sent_objs}/{obj_count} objects"
                    ));
                    last_msg_objs = sent_objs;
                    last_msg_at = Instant::now();
                }
            }
            // Final message reflects the true final count in case the
            // last batch was under the throttle threshold.
            pb_clone.set_message(format!(
                "Writing objects: {sent_objs}/{obj_count} objects"
            ));
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
        // Session finished cleanly — drop the sidecar so the next
        // push doesn't try to resume a stale id.
        clear_session(ws);
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

/// Walk the snapshot chain from `tip` and collect every reachable
/// object hash, stopping at `stop_hash`.
///
/// ## Why this is a perf hot path
///
/// On a first push (remote empty), the walk has to enumerate the
/// closure of every snapshot — for a mid-size UE project that's
/// 100k-500k objects. A naive recursive walk does a sequential disk
/// read + zstd-decompress + bincode-parse per object, which on a
/// mechanical disk was taking the CLI 1-2 minutes between "forge
/// push" and the first progress line.
///
/// Two optimisations here:
///
/// 1. **Tree BFS in rayon-parallel levels.** Snapshot-chain traversal
///    is sequential (parent pointers form a chain, not a tree), but
///    once we have the root trees, every tree read is independent.
///    Each BFS level reads the current frontier's trees in parallel
///    via `par_iter`, then advances to the next level. Scales with
///    the drive's parallel-read throughput.
///
/// 2. **Skip `get_chunked_blob` for non-chunk-manifest files.**
///    forge-core only writes a ChunkedBlob manifest when a file is
///    either ≥ `SMALL_FILE_THRESHOLD` (1 MiB) or has an extension
///    that triggers semantic chunking (.uasset/.umap/.uexp/.ubulk).
///    Every other file is a raw blob with no manifest, so probing
///    with `get_chunked_blob` pays a decompress+parse on every entry
///    and always returns Err. Pre-filter the file entries by size
///    and extension before probing.
fn collect_snapshot_objects(
    ws: &Workspace,
    tip: &ForgeHash,
    stop_hash: &[u8],
    objects: &mut Vec<ForgeHash>,
    seen: &mut HashSet<ForgeHash>,
) -> Result<()> {
    use rayon::prelude::*;

    if tip.is_zero() || tip.as_bytes().as_slice() == stop_hash || !seen.insert(*tip) {
        return Ok(());
    }

    // Phase 1: walk the snapshot chain sequentially. Collects tree
    // roots for the parallel phase below. The chain is usually short
    // (hundreds of commits max) so parallelising here adds overhead
    // without saving wall-clock.
    let mut tree_roots: Vec<ForgeHash> = Vec::new();
    let mut pending_snapshots = vec![*tip];
    while let Some(h) = pending_snapshots.pop() {
        if h.is_zero() || h.as_bytes().as_slice() == stop_hash {
            continue;
        }
        if !seen.insert(h) && h != *tip {
            // `insert(*tip)` at function entry already returned true for `tip`,
            // so `h == *tip` here means the sentinel re-insertion we want to
            // allow through.
            continue;
        }
        objects.push(h);
        let snapshot = ws.object_store.get_snapshot(&h)?;
        if seen.insert(snapshot.tree) {
            tree_roots.push(snapshot.tree);
        }
        for parent in &snapshot.parents {
            pending_snapshots.push(*parent);
        }
    }

    // Phase 2: BFS the tree DAG level-by-level with rayon-parallel
    // reads at each level. A level is a batch of tree hashes that
    // are all novel (not yet in `seen`) and therefore safe to read
    // concurrently without racing to insert duplicates.
    let mut frontier: Vec<ForgeHash> = tree_roots;
    while !frontier.is_empty() {
        // Read every tree at this level in parallel.
        let level_results: Vec<Result<Vec<forge_core::object::tree::TreeEntry>>> = frontier
            .par_iter()
            .map(|h| Ok(ws.object_store.get_tree(h)?.entries))
            .collect();
        objects.extend_from_slice(&frontier);

        // Flatten into directory children (next frontier) + file
        // entries (candidates for ChunkedBlob probe). We can't push
        // into `seen` from inside par_iter without a mutex, so we
        // do it here on the already-collected child list.
        let mut next_dirs: Vec<ForgeHash> = Vec::new();
        let mut file_candidates: Vec<(ForgeHash, u64, String)> = Vec::new();
        for entries in level_results {
            for entry in entries? {
                match entry.kind {
                    forge_core::object::tree::EntryKind::Directory => {
                        if seen.insert(entry.hash) {
                            next_dirs.push(entry.hash);
                        }
                    }
                    _ => {
                        if seen.insert(entry.hash) {
                            objects.push(entry.hash);
                            if could_be_chunked_manifest(entry.size, &entry.name) {
                                file_candidates.push((entry.hash, entry.size, entry.name));
                            }
                        }
                    }
                }
            }
        }

        // Parallel-probe ChunkedBlob manifests for the files that
        // might actually be manifests. Collect their chunk hashes;
        // dedupe into `seen` back on the main thread.
        let chunk_hashes: Vec<Vec<ForgeHash>> = file_candidates
            .par_iter()
            .map(|(h, _, _)| {
                ws.object_store
                    .get_chunked_blob(h)
                    .map(|cb| cb.chunks.into_iter().map(|c| c.hash).collect())
                    .unwrap_or_default()
            })
            .collect();
        for chunks in chunk_hashes {
            for h in chunks {
                if seen.insert(h) {
                    objects.push(h);
                }
            }
        }

        frontier = next_dirs;
    }

    Ok(())
}

/// Return true if a tree-entry file could have been written as a
/// `ChunkedBlob` manifest by forge-core. Mirrors the logic in
/// `forge_core::chunk::chunk_file_with_hint`: small files below the
/// 1 MiB threshold that aren't UE-semantic-chunk candidates are
/// always raw blobs with no manifest, so there's nothing to probe.
fn could_be_chunked_manifest(size: u64, name: &str) -> bool {
    if size >= forge_core::chunk::SMALL_FILE_THRESHOLD {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".uasset")
        || lower.ends_with(".umap")
        || lower.ends_with(".uexp")
        || lower.ends_with(".ubulk")
}
