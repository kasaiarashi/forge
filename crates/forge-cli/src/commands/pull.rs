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

        // Pre-create shard directories so the eventual rename skips
        // create_dir_all.
        ws.object_store.chunks.ensure_shard_dirs()?;

        // Phase 3e.3b — partial files live under `objects/_pull_tmp/`.
        // A crashed or network-interrupted pull leaves sibling
        // `<hash>.partial` files on disk; the next pull stats them
        // to populate `want_objects.start_offset` so the server
        // replays only the suffix the client still needs. This takes
        // the "100 GiB push survives network kill with zero redundant
        // bytes on resume" phrase in the Phase-3 plan and extends it
        // to pulls.
        let pull_tmp_dir = ws
            .object_store
            .chunks
            .root()
            .join("_pull_tmp");
        std::fs::create_dir_all(&pull_tmp_dir)?;

        const BATCH_SIZE: usize = 5000;
        for batch_chunk in missing.chunks(BATCH_SIZE) {
            // Probe `.partial` sizes for every requested hash. A
            // missing / zero-length partial gets start_offset = 0
            // which the server treats identically to the legacy
            // want_hashes path.
            let want_objects: Vec<forge_proto::forge::WantObject> = batch_chunk
                .iter()
                .map(|fh| {
                    let hex = fh.to_hex();
                    let partial = pull_tmp_dir.join(format!("{hex}.partial"));
                    let start_offset = std::fs::metadata(&partial)
                        .ok()
                        .map(|m| m.len())
                        .unwrap_or(0);
                    forge_proto::forge::WantObject {
                        hash: fh.as_bytes().to_vec(),
                        start_offset,
                    }
                })
                .collect();

            let mut stream = client
                .pull_objects(PullRequest {
                    // Leave `want_hashes` empty — server picks the
                    // resume-aware list when it's non-empty.
                    want_hashes: Vec::new(),
                    repo: repo_name.to_string(),
                    want_objects,
                })
                .await?
                .into_inner();

            // Per-object rolling state. `current_data` accumulates
            // for metadata objects only (small — snapshot / tree /
            // chunked-blob manifests) so the child-walker has bytes
            // to parse. Leaf chunks (pre_compressed = false here —
            // raw blob bodies) skip the memory accumulator entirely;
            // their bytes live on disk in `.partial` and that's
            // enough.
            let mut current_data: Vec<u8> = Vec::new();
            let mut current_hash: Option<Vec<u8>> = None;
            let mut current_type: u32 = 0;
            let mut current_partial: Option<std::fs::File> = None;
            let mut current_partial_path: Option<std::path::PathBuf> = None;

            while let Some(chunk) = stream.message().await? {
                if current_hash.as_ref() != Some(&chunk.hash) {
                    // New object frame. Close any previously-open
                    // partial (should already be closed on is_last)
                    // and open / append-open the one for this hash.
                    current_partial.take();
                    let hex = hex::encode(&chunk.hash);
                    let partial_path = pull_tmp_dir.join(format!("{hex}.partial"));
                    let file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&partial_path)?;
                    // Seed `current_data` from the existing partial
                    // bytes ONLY when this is a metadata object so
                    // the child walker sees the full compressed
                    // payload. For leaves we keep current_data
                    // empty — saves the read for multi-GB blobs.
                    if chunk.object_type == 1 && chunk.offset > 0 {
                        current_data = std::fs::read(&partial_path)?;
                    } else {
                        current_data.clear();
                    }
                    current_hash = Some(chunk.hash.clone());
                    current_type = chunk.object_type;
                    current_partial = Some(file);
                    current_partial_path = Some(partial_path);
                }

                received_bytes += chunk.data.len() as u64;

                // Persist the chunk. Append-open is the cheapest
                // durability primitive we have — no fsync per chunk;
                // the `.partial` → final rename flushes the payload
                // via CloseHandle / close(2) on the OpenOptions drop.
                if let Some(f) = current_partial.as_mut() {
                    use std::io::Write;
                    f.write_all(&chunk.data)?;
                }
                if current_type == 1 {
                    current_data.extend_from_slice(&chunk.data);
                }

                if chunk.is_last {
                    // Drop the partial handle first so the rename
                    // below never races an open file handle on
                    // Windows.
                    drop(current_partial.take());

                    let hash_hex = hex::encode(&chunk.hash);
                    let forge_hash = ForgeHash::from_hex(&hash_hex)?;
                    let pre_compressed = current_type == 1;

                    if pre_compressed {
                        if let Ok(decompressed) = forge_core::compress::decompress(&current_data) {
                            walk_children_from_data(&decompressed, &mut want);
                        }
                    }
                    // Leaf bytes live on disk only — no walking.

                    // Rename `.partial` into the live shard. If the
                    // final path somehow already exists (dedup
                    // race with another pull), drop the partial.
                    let final_path = ws
                        .object_store
                        .chunks
                        .root()
                        .join(&hash_hex[..2])
                        .join(&hash_hex[2..]);
                    let partial_path = current_partial_path
                        .take()
                        .expect("is_last without a partial path");
                    if final_path.exists() {
                        let _ = std::fs::remove_file(&partial_path);
                    } else {
                        if let Some(parent) = final_path.parent() {
                            std::fs::create_dir_all(parent).ok();
                        }
                        std::fs::rename(&partial_path, &final_path)?;
                    }

                    // Drop the memory accumulator so the next object's
                    // metadata doesn't see stale bytes.
                    current_data.clear();
                    current_hash = None;

                    received += 1;
                    let elapsed = start.elapsed().as_secs_f64().max(0.001);
                    let speed = received_bytes as f64 / elapsed;
                    pb.set_message(format_receive_progress(received, received_bytes, speed));

                    // Surface `forge_hash` in a non-warning way — the
                    // variable is read above via `hash_hex` / `final_path`,
                    // this line keeps it alive against an
                    // unused-variable lint if the logic ever shrinks.
                    let _ = forge_hash;
                }
            }
        }

        // Best-effort: clear stray `.partial` files that made it to
        // the rename stage via a prior successful pull but were
        // orphaned by a crash between rename and cleanup. Safe to
        // remove — any live partial is actively being written by
        // THIS process.
        if let Ok(entries) = std::fs::read_dir(&pull_tmp_dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("partial") {
                    // Only delete when the final object already
                    // lives in the shard tree — otherwise a future
                    // pull could reuse the partial to resume.
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        if stem.len() == 64 {
                            let final_path = ws
                                .object_store
                                .chunks
                                .root()
                                .join(&stem[..2])
                                .join(&stem[2..]);
                            if final_path.exists() {
                                let _ = std::fs::remove_file(&p);
                            }
                        }
                    }
                }
            }
        }
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
