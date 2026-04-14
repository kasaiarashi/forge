use anyhow::{bail, Result};
use chrono::Utc;
use forge_core::diff::flatten_tree;
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::workspace::Workspace;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Stash {
    message: String,
    parent: String,
    timestamp: String,
    entries: Vec<StashEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StashEntry {
    path: String,
    hash: String,
    size: u64,
    is_chunked: bool,
    object_hash: String,
    mtime_secs: i64,
    mtime_nanos: u32,
}

pub fn run(action: Option<String>, message: Option<String>) -> Result<()> {
    let action = action.unwrap_or_else(|| "push".to_string());

    match action.as_str() {
        "push" => stash_push(message),
        "pop" => stash_pop(true),
        "apply" => stash_pop(false),
        "list" => stash_list(),
        "drop" => stash_drop(),
        "show" => stash_show(),
        other => bail!("Unknown stash action: '{}'. Use push, pop, apply, list, show, or drop.", other),
    }
}

fn stash_dir(ws: &Workspace) -> std::path::PathBuf {
    ws.forge_dir().join("stash")
}

fn next_stash_id(ws: &Workspace) -> Result<u32> {
    let dir = stash_dir(ws);
    if !dir.exists() {
        return Ok(0);
    }
    let mut max_id: Option<u32> = None;
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            if let Some(num_str) = name.strip_suffix(".json") {
                if let Ok(n) = num_str.parse::<u32>() {
                    max_id = Some(max_id.map_or(n, |m: u32| m.max(n)));
                }
            }
        }
    }
    Ok(max_id.map_or(0, |m| m + 1))
}

fn list_stash_ids(ws: &Workspace) -> Result<Vec<u32>> {
    let dir = stash_dir(ws);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            if let Some(num_str) = name.strip_suffix(".json") {
                if let Ok(n) = num_str.parse::<u32>() {
                    ids.push(n);
                }
            }
        }
    }
    ids.sort();
    Ok(ids)
}

fn stash_push(message: Option<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let index_path = ws.forge_dir().join("index");
    let index = Index::load(&index_path)?;

    let head_hash = ws.head_snapshot()?;

    // Find entries that differ from HEAD tree (modified or staged).
    let head_flat = if head_hash.is_zero() {
        std::collections::BTreeMap::new()
    } else {
        let snap = ws.object_store.get_snapshot(&head_hash)?;
        let tree = ws.object_store.get_tree(&snap.tree)?;
        let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();
        flatten_tree(&tree, "", &get_tree)
    };

    // Collect entries that are staged or have different content from HEAD.
    let mut stash_entries = Vec::new();
    for (path, entry) in &index.entries {
        if entry.staged {
            // Staged entries are always stashed (content already in object store via add).
            stash_entries.push(StashEntry {
                path: path.clone(),
                hash: entry.hash.to_hex(),
                size: entry.size,
                is_chunked: entry.is_chunked,
                object_hash: entry.object_hash.to_hex(),
                mtime_secs: entry.mtime_secs,
                mtime_nanos: entry.mtime_nanos,
            });
            continue;
        }

        // For unstaged entries, check if the working tree file differs from the index.
        let rel_disk = path.replace('/', std::path::MAIN_SEPARATOR_STR);
        let abs_path = ws.root.join(&rel_disk);
        if !abs_path.exists() {
            // File deleted on disk but still in index — stash the deletion.
            if head_flat.contains_key(path) {
                stash_entries.push(StashEntry {
                    path: path.clone(),
                    hash: ForgeHash::ZERO.to_hex(),
                    size: 0,
                    is_chunked: false,
                    object_hash: ForgeHash::ZERO.to_hex(),
                    mtime_secs: 0,
                    mtime_nanos: 0,
                });
            }
            continue;
        }

        // Fast-path: compare mtime + size with index.
        let metadata = std::fs::metadata(&abs_path)?;
        let mtime = metadata
            .modified()?
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        if mtime.as_secs() as i64 == entry.mtime_secs
            && mtime.subsec_nanos() == entry.mtime_nanos
            && metadata.len() == entry.size
        {
            continue; // Unchanged
        }

        // Re-hash to confirm content change.
        let data = std::fs::read(&abs_path)?;
        let disk_hash = ForgeHash::from_bytes(&data);
        if disk_hash == entry.hash {
            continue; // Content identical despite mtime change
        }

        // Working tree file differs — store the content and record it.
        let object_hash = ws.object_store.put_blob_data(&data)?;
        stash_entries.push(StashEntry {
            path: path.clone(),
            hash: disk_hash.to_hex(),
            size: data.len() as u64,
            is_chunked: false,
            object_hash: object_hash.to_hex(),
            mtime_secs: mtime.as_secs() as i64,
            mtime_nanos: mtime.subsec_nanos(),
        });
    }

    if stash_entries.is_empty() {
        bail!("No changes to stash.");
    }

    let msg = message.unwrap_or_else(|| {
        format!("WIP on {}", head_hash.short())
    });

    let stash = Stash {
        message: msg.clone(),
        parent: head_hash.to_hex(),
        timestamp: Utc::now().to_rfc3339(),
        entries: stash_entries,
    };

    // Save stash file.
    let dir = stash_dir(&ws);
    std::fs::create_dir_all(&dir)?;
    let id = next_stash_id(&ws)?;
    let stash_path = dir.join(format!("{}.json", id));
    let json = serde_json::to_string_pretty(&stash)?;
    std::fs::write(&stash_path, json)?;

    // Restore working tree files to HEAD state.
    for se in &stash.entries {
        let rel_disk = se.path.replace('/', std::path::MAIN_SEPARATOR_STR);
        let abs_path = ws.root.join(&rel_disk);
        match head_flat.get(&se.path) {
            Some((hash, _)) => {
                // File exists in HEAD — restore it.
                let content = read_blob_content(&ws, hash)?;
                if let Some(parent) = abs_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&abs_path, &content)?;
            }
            None => {
                // File is new (not in HEAD) — remove it from working tree.
                if abs_path.exists() {
                    std::fs::remove_file(&abs_path)?;
                }
            }
        }
    }

    // Reset index to HEAD tree state (all unstaged).
    let mut new_index = Index::default();
    for (path, (hash, size)) in &head_flat {
        let rel_disk = path.replace('/', std::path::MAIN_SEPARATOR_STR);
        let abs_path = ws.root.join(&rel_disk);
        let (mtime_secs, mtime_nanos) = if abs_path.exists() {
            mtime_of(&abs_path)
        } else {
            (0, 0)
        };
        new_index.set(path.clone(), IndexEntry {
            hash: *hash,
            size: *size,
            mtime_secs,
            mtime_nanos,
            staged: false,
            is_chunked: false,
            object_hash: *hash,
        });
    }
    new_index.save(&index_path)?;

    println!("Saved working directory to stash@{{{}}}: {}", id, msg);
    Ok(())
}

fn stash_pop(remove: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let index_path = ws.forge_dir().join("index");

    let ids = list_stash_ids(&ws)?;
    if ids.is_empty() {
        bail!("No stashes found.");
    }

    let latest_id = *ids.last().expect("checked non-empty above");
    let stash_path = stash_dir(&ws).join(format!("{}.json", latest_id));
    let json = std::fs::read_to_string(&stash_path)?;
    let stash: Stash = serde_json::from_str(&json)?;

    // Apply stash entries: restore file contents to working tree and update index.
    let mut index = Index::load(&index_path)?;
    for se in &stash.entries {
        let hash = ForgeHash::from_hex(&se.hash)?;
        let object_hash = ForgeHash::from_hex(&se.object_hash)?;
        let rel_disk = se.path.replace('/', std::path::MAIN_SEPARATOR_STR);
        let abs_path = ws.root.join(&rel_disk);

        if hash == ForgeHash::ZERO {
            // Stashed deletion — remove the file.
            if abs_path.exists() {
                std::fs::remove_file(&abs_path)?;
            }
            index.entries.remove(&se.path);
        } else {
            // Restore file content from object store.
            let content = read_blob_content(&ws, &object_hash)?;
            if let Some(parent) = abs_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&abs_path, &content)?;

            // Update mtime to reflect the newly written file.
            let new_mtime = mtime_of(&abs_path);
            index.set(se.path.clone(), IndexEntry {
                hash,
                size: se.size,
                mtime_secs: new_mtime.0,
                mtime_nanos: new_mtime.1,
                staged: true,
                is_chunked: se.is_chunked,
                object_hash,
            });
        }
    }
    index.save(&index_path)?;

    if remove {
        std::fs::remove_file(&stash_path)?;
        println!("Applied and dropped stash@{{{}}}.", latest_id);
    } else {
        println!("Applied stash@{{{}}} (kept in stash list).", latest_id);
    }

    Ok(())
}

fn stash_list() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    let ids = list_stash_ids(&ws)?;
    if ids.is_empty() {
        println!("No stashes.");
        return Ok(());
    }

    for id in &ids {
        let path = stash_dir(&ws).join(format!("{}.json", id));
        if let Ok(json) = std::fs::read_to_string(&path) {
            if let Ok(stash) = serde_json::from_str::<Stash>(&json) {
                println!(
                    "stash@{{{}}}: {} ({} file(s))",
                    id,
                    stash.message,
                    stash.entries.len()
                );
            }
        }
    }

    Ok(())
}

fn stash_drop() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    let ids = list_stash_ids(&ws)?;
    if ids.is_empty() {
        bail!("No stashes to drop.");
    }

    let latest_id = *ids.last().expect("checked non-empty above");
    let stash_path = stash_dir(&ws).join(format!("{}.json", latest_id));
    std::fs::remove_file(&stash_path)?;
    println!("Dropped stash@{{{}}}.", latest_id);

    Ok(())
}

fn stash_show() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    let ids = list_stash_ids(&ws)?;
    if ids.is_empty() {
        bail!("No stashes found.");
    }

    let latest_id = *ids.last().expect("checked non-empty above");
    let stash_path = stash_dir(&ws).join(format!("{}.json", latest_id));
    let data = std::fs::read_to_string(&stash_path)?;
    let stash: Stash = serde_json::from_str(&data)?;

    println!("stash@{{{}}}: {}", latest_id, stash.message);
    println!();
    for entry in &stash.entries {
        println!("  {:<12}{}", "modified:", entry.path);
    }
    println!();
    println!(" {} file(s) stashed", stash.entries.len());

    Ok(())
}

/// Read blob content, handling both small and chunked blobs.
fn read_blob_content(ws: &Workspace, object_hash: &ForgeHash) -> Result<Vec<u8>> {
    let data = ws
        .object_store
        .chunks
        .get(object_hash)
        .map_err(|e| anyhow::anyhow!("Failed to read object {}: {}", object_hash.short(), e))?;

    if data.is_empty() {
        return Ok(data);
    }

    if data[0] == 2 {
        let manifest: forge_core::object::blob::ChunkedBlob = bincode::deserialize(&data[1..])
            .map_err(|e| anyhow::anyhow!("Failed to deserialize manifest: {}", e))?;
        let content = forge_core::chunk::reassemble_chunks(&manifest, |h| {
            ws.object_store.chunks.get(h).ok()
        })
        .ok_or_else(|| anyhow::anyhow!("Failed to reassemble chunked blob"))?;
        Ok(content)
    } else {
        Ok(data)
    }
}

fn mtime_of(path: &std::path::Path) -> (i64, u32) {
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(mtime) = meta.modified() {
            let dur = mtime
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default();
            return (dur.as_secs() as i64, dur.subsec_nanos());
        }
    }
    (0, 0)
}
