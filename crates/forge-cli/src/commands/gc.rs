use anyhow::Result;
use forge_core::hash::ForgeHash;
use forge_core::object::blob::ChunkedBlob;
use forge_core::object::tree::{EntryKind, Tree};
use forge_core::workspace::Workspace;
use std::collections::HashSet;

pub fn run(dry_run: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    // Phase 1: Collect all reachable object hashes.
    let mut reachable: HashSet<ForgeHash> = HashSet::new();

    // Collect all ref tips (branches + tags).
    let mut tips: Vec<ForgeHash> = Vec::new();

    for branch in ws.list_branches()? {
        if let Ok(tip) = ws.get_branch_tip(&branch) {
            if !tip.is_zero() {
                tips.push(tip);
            }
        }
    }

    let tags_dir = ws.forge_dir().join("refs").join("tags");
    if tags_dir.exists() {
        for entry in std::fs::read_dir(&tags_dir)? {
            let entry = entry?;
            if let Ok(hex) = std::fs::read_to_string(entry.path()) {
                if let Ok(hash) = ForgeHash::from_hex(hex.trim()) {
                    if !hash.is_zero() {
                        tips.push(hash);
                    }
                }
            }
        }
    }

    // Also include stash parents and referenced objects.
    let stash_dir = ws.forge_dir().join("stash");
    if stash_dir.exists() {
        for entry in std::fs::read_dir(&stash_dir)? {
            let entry = entry?;
            if let Ok(json) = std::fs::read_to_string(entry.path()) {
                if let Ok(stash) = serde_json::from_str::<serde_json::Value>(&json) {
                    // Mark the parent commit as reachable.
                    if let Some(parent) = stash.get("parent").and_then(|v| v.as_str()) {
                        if let Ok(h) = ForgeHash::from_hex(parent) {
                            if !h.is_zero() {
                                tips.push(h);
                            }
                        }
                    }
                    // Mark stashed object hashes as reachable.
                    if let Some(entries) = stash.get("entries").and_then(|v| v.as_array()) {
                        for se in entries {
                            if let Some(oh) = se.get("object_hash").and_then(|v| v.as_str()) {
                                if let Ok(h) = ForgeHash::from_hex(oh) {
                                    if !h.is_zero() {
                                        reachable.insert(h);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if tips.is_empty() {
        println!("No refs found — nothing to do.");
        return Ok(());
    }

    // Walk commit history from each tip and mark all reachable objects.
    let mut commit_queue: Vec<ForgeHash> = tips;
    let mut visited_commits: HashSet<ForgeHash> = HashSet::new();

    while let Some(commit_hash) = commit_queue.pop() {
        if !visited_commits.insert(commit_hash) {
            continue;
        }
        reachable.insert(commit_hash);

        let snapshot = match ws.object_store.get_snapshot(&commit_hash) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Mark tree and all its contents.
        mark_tree_reachable(&ws, &snapshot.tree, &mut reachable);

        // Enqueue parents.
        for parent in &snapshot.parents {
            if !parent.is_zero() && !visited_commits.contains(parent) {
                commit_queue.push(*parent);
            }
        }
    }

    // Phase 2: Enumerate all objects on disk.
    let objects_dir = ws.forge_dir().join("objects");
    let mut all_objects: Vec<ForgeHash> = Vec::new();

    if objects_dir.exists() {
        for shard in std::fs::read_dir(&objects_dir)? {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            let shard_name = shard.file_name();
            let shard_hex = shard_name.to_string_lossy();
            if shard_hex.len() != 2 {
                continue;
            }

            for obj in std::fs::read_dir(shard.path())? {
                let obj = obj?;
                let obj_name = obj.file_name();
                let obj_hex = obj_name.to_string_lossy();
                // Skip temp files.
                if obj_hex.ends_with(".tmp") {
                    continue;
                }
                let full_hex = format!("{}{}", shard_hex, obj_hex);
                if let Ok(hash) = ForgeHash::from_hex(&full_hex) {
                    all_objects.push(hash);
                }
            }
        }
    }

    // Phase 3: Compute unreachable and remove.
    let total = all_objects.len();
    let unreachable: Vec<ForgeHash> = all_objects
        .into_iter()
        .filter(|h| !reachable.contains(h))
        .collect();

    let unreachable_count = unreachable.len();

    if unreachable_count == 0 {
        println!(
            "All {} objects are reachable. Nothing to prune.",
            total
        );
        return Ok(());
    }

    // Calculate space to be freed.
    let mut bytes_freed: u64 = 0;
    for hash in &unreachable {
        let hex = hash.to_hex();
        let path = objects_dir.join(&hex[..2]).join(&hex[2..]);
        if let Ok(meta) = std::fs::metadata(&path) {
            bytes_freed += meta.len();
        }
    }

    if dry_run {
        println!(
            "Would prune {} of {} objects ({}).",
            unreachable_count,
            total,
            format_bytes(bytes_freed)
        );
    } else {
        for hash in &unreachable {
            ws.object_store.chunks.delete(hash)?;
        }

        // Remove empty shard directories.
        if let Ok(entries) = std::fs::read_dir(&objects_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let _ = std::fs::remove_dir(entry.path()); // Only succeeds if empty.
                }
            }
        }

        println!(
            "Pruned {} of {} objects, freed {}.",
            unreachable_count,
            total,
            format_bytes(bytes_freed)
        );
    }

    Ok(())
}

/// Recursively mark a tree and all its contents as reachable.
fn mark_tree_reachable(ws: &Workspace, tree_hash: &ForgeHash, reachable: &mut HashSet<ForgeHash>) {
    if !reachable.insert(*tree_hash) {
        return; // Already visited.
    }

    let tree: Tree = match ws.object_store.get_tree(tree_hash) {
        Ok(t) => t,
        Err(_) => return,
    };

    for entry in &tree.entries {
        match entry.kind {
            EntryKind::Directory => {
                mark_tree_reachable(ws, &entry.hash, reachable);
            }
            EntryKind::File | EntryKind::Symlink => {
                reachable.insert(entry.hash);
                // If it's a chunked blob, also mark individual chunks.
                if let Ok(data) = ws.object_store.chunks.get(&entry.hash) {
                    if !data.is_empty() && data[0] == 2 {
                        if let Ok(manifest) =
                            bincode::deserialize::<ChunkedBlob>(&data[1..])
                        {
                            for chunk in &manifest.chunks {
                                reachable.insert(chunk.hash);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}
