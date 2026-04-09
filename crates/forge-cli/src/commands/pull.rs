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
    let ws = Workspace::discover(&cwd)?;
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
    let mut total_discovered = 0u64;

    // Progress bar for receiving objects — length updated as we discover more.
    let pb = ProgressBar::new(0);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("Receiving objects: [{bar:30}] {pos}/{len} ({percent}%)")
            .unwrap()
            .progress_chars("=>-"),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    while !want.is_empty() {
        // Dedup the queue against the visited set. Anything we've already
        // walked (from disk or wire) does not get processed twice — that's
        // how the BFS terminates.
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

        // Split into "already on disk" and "missing". Both groups need
        // their children walked into `want`; only the missing group is
        // actually fetched from the server.
        let (missing, present): (Vec<ForgeHash>, Vec<ForgeHash>) = to_visit
            .into_iter()
            .partition(|fh| !ws.object_store.has(fh));

        // Walk children of objects already on disk — the resume-clone fix.
        for fh in &present {
            walk_object_children(ws, fh, &mut want);
        }

        if missing.is_empty() {
            continue;
        }

        total_discovered += missing.len() as u64;
        pb.set_length(total_discovered);

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
                })
                .await?
                .into_inner();

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

                    let computed = ForgeHash::from_bytes(&current_data);
                    if computed != forge_hash {
                        pb.finish_and_clear();
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
                    pb.set_position(received);

                    // Walk newly-fetched object's children — same helper
                    // as the already-on-disk path so the two cases stay in
                    // sync.
                    walk_object_children(ws, &forge_hash, &mut want);

                    current_data.clear();
                    current_hash = None;
                }
            }
        }
    }

    pb.finish_and_clear();
    Ok(received)
}

/// Push the children of `hash` (referenced object hashes) onto `want` so the
/// pull BFS keeps walking. Dispatch is by the 1-byte type tag stored at the
/// start of typed objects (Snapshot/Tree/ChunkedBlob); raw blobs and chunks
/// are leaves and contribute nothing. Errors during read/parse are silently
/// swallowed — a corrupt object on disk gets caught later by `read_file`
/// during checkout, where the error message is more useful to the user.
fn walk_object_children(ws: &Workspace, hash: &ForgeHash, want: &mut Vec<Vec<u8>>) {
    let data = match ws.object_store.chunks.get(hash) {
        Ok(d) => d,
        Err(_) => return,
    };
    if data.is_empty() {
        return;
    }
    let tag = data[0];
    if tag == ObjectType::Snapshot as u8 {
        if let Ok(snap) = ws.object_store.get_snapshot(hash) {
            want.push(snap.tree.as_bytes().to_vec());
            for parent in &snap.parents {
                if !parent.is_zero() {
                    want.push(parent.as_bytes().to_vec());
                }
            }
        }
    } else if tag == ObjectType::Tree as u8 {
        if let Ok(tree) = ws.object_store.get_tree(hash) {
            for entry in &tree.entries {
                want.push(entry.hash.as_bytes().to_vec());
            }
        }
    } else if tag == ObjectType::ChunkedBlob as u8 {
        if let Ok(chunked) = ws.object_store.get_chunked_blob(hash) {
            for chunk_ref in &chunked.chunks {
                want.push(chunk_ref.hash.as_bytes().to_vec());
            }
        }
    }
    // ObjectType::Blob (tag=1) and raw chunk bytes are leaves — no children.
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
