// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::Result;
use forge_core::diff::flatten_tree;
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::workspace::Workspace;
use forge_proto::forge::*;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::time::SystemTime;

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

    // Request all objects from the remote tip that we don't have locally.
    let mut want = vec![remote_tip_bytes.clone()];
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

    loop {
        if want.is_empty() {
            break;
        }

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

        total_discovered += need.len() as u64;
        pb.set_length(total_discovered);

        const BATCH_SIZE: usize = 5000;
        let batches: Vec<Vec<Vec<u8>>> = need
            .chunks(BATCH_SIZE)
            .map(|c| c.to_vec())
            .collect();

        want.clear();

        for batch in batches {
            let mut stream = client
                .pull_objects(PullRequest {
                    want_hashes: batch,
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

                    if let Ok(snapshot) = ws.object_store.get_snapshot(&forge_hash) {
                        want.push(snapshot.tree.as_bytes().to_vec());
                        for parent in &snapshot.parents {
                            if !parent.is_zero() {
                                want.push(parent.as_bytes().to_vec());
                            }
                        }
                    }

                    if let Ok(tree) = ws.object_store.get_tree(&forge_hash) {
                        for entry in &tree.entries {
                            want.push(entry.hash.as_bytes().to_vec());
                        }
                    }

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
    }

    pb.finish_and_clear();
    println!("Receiving objects: {} done.", received);

    // Fast-forward local branch.
    let old_tip = local_tip.short();
    ws.set_branch_tip(&branch, &remote_tip)?;
    println!("   {}..{} {} -> {}", old_tip, remote_tip.short(), branch, branch);

    // Checkout working tree from the new tip.
    checkout_tree(ws, &remote_tip)?;

    Ok(())
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
