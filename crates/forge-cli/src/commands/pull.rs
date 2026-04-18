// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::Result;
use forge_core::diff::flatten_tree;
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::object::ObjectType;
use forge_core::workspace::Workspace;
use forge_proto::forge::forge_service_client::ForgeServiceClient;
use forge_proto::forge::*;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::collections::HashSet;
use std::time::SystemTime;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;

use crate::client::AuthInterceptor;

/// Type alias for the authenticated forge gRPC client. Used by both pull
/// and fetch — saves spelling out the full nested generic in every
/// signature.
pub(super) type AuthedForgeClient =
    ForgeServiceClient<InterceptedService<Channel, AuthInterceptor>>;

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir()?;
    run_in(&cwd)
}

/// Explicit-cwd entry used by the Phase-4 FFI layer so a concurrent
/// caller can't race the process-wide CWD.
pub fn run_in(cwd: &std::path::Path) -> Result<()> {
    let ws = Workspace::discover(cwd)?;
    run_with_workspace(&ws)
}

/// Pull using an already-opened workspace (used by clone).
pub fn run_with_workspace(ws: &Workspace) -> Result<()> {
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
    rt.block_on(async { pull_async(ws, &server_url, &repo_name).await })
}

async fn pull_async(ws: &Workspace, server_url: &str, repo_name: &str) -> Result<()> {
    let mut client = crate::client::connect_forge(server_url).await?;

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

    let received = fetch_objects_to_tip(ws, &mut client, repo_name, &remote_tip_bytes).await?;
    println!("Receiving objects: {} done.", received);

    // Fast-forward local branch.
    let old_tip = local_tip.short();
    ws.set_branch_tip(&branch, &remote_tip)?;
    println!("   {}..{} {} -> {}", old_tip, remote_tip.short(), branch, branch);

    // Checkout working tree from the new tip.
    checkout_tree(ws, &remote_tip)?;

    Ok(())
}

/// BFS-walk the dependency graph from `tip_bytes` down to leaf chunks,
/// fetching anything missing from the server in batches. Shared by `pull`
/// and `fetch` — both need exactly this behavior, just with different
/// follow-up actions (pull advances HEAD + checks out; fetch updates
/// remote-tracking refs only).
///
/// We walk children of *every* visited object — both those we just fetched
/// AND those already on disk from a prior run. The already-on-disk case is
/// the resumable-clone fix: a previous attempt typically leaves the
/// snapshot, trees, and chunked-blob manifests written, but dies partway
/// through fetching the actual chunks. If we only walked children of
/// newly-fetched objects, resume would see "tip already present, nothing
/// to do", terminate immediately, and then crash in checkout with "failed
/// to reassemble chunked blob".
///
/// Returns the count of objects newly received from the server (zero when
/// the local store already had everything).
pub(super) async fn fetch_objects_to_tip(
    ws: &Workspace,
    client: &mut AuthedForgeClient,
    repo_name: &str,
    tip_bytes: &[u8],
) -> Result<u64> {
    let mut want: Vec<Vec<u8>> = vec![tip_bytes.to_vec()];
    let mut visited: HashSet<ForgeHash> = HashSet::new();
    let mut received = 0u64;
    let mut received_bytes = 0u64;

    // Use a byte-based progress bar without a total — the BFS discovers
    // objects in waves (snapshots → trees → blobs → chunks) so we can't
    // know the total upfront. Showing bytes + speed avoids the misleading
    // "almost done" resets that a percentage bar would cause.
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} Receiving objects: {msg}")
            .unwrap(),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    let start = std::time::Instant::now();

    while !want.is_empty() {
        let mut to_visit: Vec<ForgeHash> = Vec::with_capacity(want.len());
        for h in want.drain(..) {
            if let Ok(fh) = ForgeHash::from_hex(&hex::encode(&h)) {
                if visited.insert(fh) {
                    to_visit.push(fh);
                }
            }
        }

        if to_visit.is_empty() {
            continue;
        }

        let (missing, present): (Vec<ForgeHash>, Vec<ForgeHash>) = to_visit
            .into_iter()
            .partition(|fh| !ws.object_store.has(fh));

        for fh in &present {
            walk_object_children(ws, fh, &mut want);
        }

        if missing.is_empty() {
            continue;
        }

        // Pre-create shard directories so writes skip create_dir_all.
        ws.object_store.chunks.ensure_shard_dirs()?;

        // Spawn background writer threads (same pattern as push).
        // Bounded channel limits memory to ~256 objects in flight
        // regardless of project size.
        let (write_tx, write_rx) =
            crossbeam_channel::bounded::<(ForgeHash, Vec<u8>, bool)>(8);
        let write_rx = std::sync::Arc::new(write_rx);
        let num_writers = rayon::current_num_threads().min(8);
        let store = ws.object_store.chunks.clone();
        let write_error: std::sync::Arc<std::sync::Mutex<Option<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let mut writer_handles = Vec::with_capacity(num_writers);
        for _ in 0..num_writers {
            let rx = std::sync::Arc::clone(&write_rx);
            let s = store.clone();
            let err = std::sync::Arc::clone(&write_error);
            writer_handles.push(std::thread::spawn(move || {
                while let Ok((hash, data, pre_compressed)) = rx.recv() {
                    let result: Result<(), _> = if pre_compressed {
                        s.put_raw_direct(&hash, &data)
                    } else {
                        s.put(&hash, &data).map(|_| ())
                    };
                    if let Err(e) = result {
                        let mut guard = err.lock().unwrap();
                        if guard.is_none() {
                            *guard = Some(e.to_string());
                        }
                        break;
                    }
                }
            }));
        }

        const BATCH_SIZE: usize = 5000;
        for batch_chunk in missing.chunks(BATCH_SIZE) {
            let batch_bytes: Vec<Vec<u8>> = batch_chunk
                .iter()
                .map(|fh| fh.as_bytes().to_vec())
                .collect();

            let mut stream = client
                .pull_objects(PullRequest {
                    want_hashes: batch_bytes,
                    repo: repo_name.to_string(),
                    // Phase 3e.3 adds the resume-aware `want_objects`
                    // field. Leaving it empty means the server derives
                    // start_offset = 0 for every hash (legacy path).
                    // Client-side partial-file persistence that would
                    // populate this field is Phase 3e.3b.
                    want_objects: Vec::new(),
                })
                .await?
                .into_inner();

            let mut current_data = Vec::new();
            let mut current_hash: Option<Vec<u8>> = None;
            let mut current_type: u32 = 0;

            while let Some(chunk) = stream.message().await? {
                if current_hash.as_ref() != Some(&chunk.hash) {
                    current_data.clear();
                    current_hash = Some(chunk.hash.clone());
                    current_type = chunk.object_type;
                }

                received_bytes += chunk.data.len() as u64;
                current_data.extend_from_slice(&chunk.data);

                if chunk.is_last {
                    let hash_hex = hex::encode(&chunk.hash);
                    let forge_hash = ForgeHash::from_hex(&hash_hex)?;
                    let pre_compressed = current_type == 1;

                    // Walk children from in-memory data before handing off
                    // to writer. Only metadata objects (snapshot/tree/chunked
                    // blob) need decompression; raw chunks are leaves and
                    // skip this entirely — so memory stays bounded.
                    if pre_compressed {
                        if let Ok(decompressed) = forge_core::compress::decompress(&current_data) {
                            walk_children_from_data(&decompressed, &mut want);
                        }
                    } else {
                        walk_children_from_data(&current_data, &mut want);
                    }

                    // Hand off to writer threads — bounded channel provides
                    // backpressure so memory stays at ~256 objects max.
                    let data = std::mem::take(&mut current_data);
                    write_tx
                        .send((forge_hash, data, pre_compressed))
                        .map_err(|_| anyhow::anyhow!("writer thread crashed"))?;

                    received += 1;
                    let elapsed = start.elapsed().as_secs_f64().max(0.001);
                    let speed = received_bytes as f64 / elapsed;
                    pb.set_message(format_receive_progress(received, received_bytes, speed));

                    current_hash = None;
                }
            }

            // Check for write errors between batches.
            if let Some(e) = write_error.lock().unwrap().take() {
                anyhow::bail!("write error: {}", e);
            }
        }

        // Signal writers to finish and wait.
        drop(write_tx);
        for h in writer_handles {
            let _ = h.join();
        }
        if let Some(e) = write_error.lock().unwrap().take() {
            anyhow::bail!("write error: {}", e);
        };
    }

    let elapsed = start.elapsed().as_secs_f64().max(0.001);
    let speed = received_bytes as f64 / elapsed;
    pb.finish_with_message(format!(
        "{} objects, {} received ({}/s), done.",
        received,
        format_bytes(received_bytes),
        format_bytes(speed as u64),
    ));
    Ok(received)
}

/// Walk children from decompressed in-memory data (no disk I/O).
fn walk_children_from_data(data: &[u8], want: &mut Vec<Vec<u8>>) {
    if data.len() < 2 {
        return;
    }
    let tag = data[0];
    // Skip the 1-byte type tag for bincode deserialization.
    let payload = &data[1..];
    if tag == ObjectType::Snapshot as u8 {
        if let Ok(snap) = bincode::deserialize::<forge_core::object::snapshot::Snapshot>(payload) {
            want.push(snap.tree.as_bytes().to_vec());
            for parent in &snap.parents {
                if !parent.is_zero() {
                    want.push(parent.as_bytes().to_vec());
                }
            }
        }
    } else if tag == ObjectType::Tree as u8 {
        if let Ok(tree) = bincode::deserialize::<forge_core::object::tree::Tree>(payload) {
            for entry in &tree.entries {
                want.push(entry.hash.as_bytes().to_vec());
            }
        }
    } else if tag == ObjectType::ChunkedBlob as u8 {
        if let Ok(chunked) = bincode::deserialize::<forge_core::object::blob::ChunkedBlob>(payload) {
            for chunk_ref in &chunked.chunks {
                want.push(chunk_ref.hash.as_bytes().to_vec());
            }
        }
    }
}

/// Walk children from disk (used for resume — objects already on disk).
fn walk_object_children(ws: &Workspace, hash: &ForgeHash, want: &mut Vec<Vec<u8>>) {
    let data = match ws.object_store.chunks.get(hash) {
        Ok(d) => d,
        Err(_) => return,
    };
    walk_children_from_data(&data, want);
}

/// Write the commit's tree contents into the working directory and update the index.
/// Uses rayon to read/decompress objects in parallel, then writes files sequentially.
fn checkout_tree(ws: &Workspace, commit_hash: &ForgeHash) -> Result<()> {
    let snap = ws.object_store.get_snapshot(commit_hash)?;
    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();
    let tree = ws.object_store.get_tree(&snap.tree)?;
    let file_map = flatten_tree(&tree, "", &get_tree);

    let mut index = Index::load(&ws.forge_dir().join("index"))?;

    // Remove files not in the target tree.
    let old_paths: Vec<String> = index.entries.keys().cloned().collect();
    for path in &old_paths {
        if !file_map.contains_key(path) {
            let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if abs_path.exists() {
                let _ = std::fs::remove_file(&abs_path);
            }
            index.remove(path);
        }
    }

    let total = file_map.len() as u64;
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("Checking out files: [{bar:30}] {pos}/{len} ({percent}%)")
            .unwrap()
            .progress_chars("=>-"),
    );

    // Collect entries for parallel read.
    let entries: Vec<(String, ForgeHash, u64)> = file_map
        .iter()
        .map(|(p, (h, s))| (p.clone(), *h, *s))
        .collect();

    // Parallel read + decompress objects.
    let read_results: Vec<Result<(String, Vec<u8>, ForgeHash, u64)>> = entries
        .par_iter()
        .map(|(path, hash, size)| {
            let content = ws.object_store.read_file(hash)?;
            Ok((path.clone(), content, *hash, *size))
        })
        .collect();

    // Sequential write to disk + index update.
    for result in read_results {
        let (path, content, obj_hash, size) = result?;
        let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = abs_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&abs_path, &content)?;

        let mtime = std::fs::metadata(&abs_path)?
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();

        index.set(
            path,
            IndexEntry {
                hash: ForgeHash::from_bytes(&content),
                size,
                mtime_secs: mtime.as_secs() as i64,
                mtime_nanos: mtime.subsec_nanos(),
                staged: false,
                is_chunked: false,
                object_hash: obj_hash,
            },
        );

        pb.inc(1);
    }

    pb.finish_and_clear();
    println!("Checking out files: {} done.", total);

    index.save(&ws.forge_dir().join("index"))?;
    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.2} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.2} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn format_receive_progress(objects: u64, bytes: u64, speed: f64) -> String {
    format!(
        "{} objects, {} ({}/s)",
        objects,
        format_bytes(bytes),
        format_bytes(speed as u64),
    )
}
