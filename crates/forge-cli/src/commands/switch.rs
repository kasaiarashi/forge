use anyhow::{bail, Result};
use forge_core::diff::{diff_maps, flatten_tree, DiffEntry};
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::workspace::{HeadRef, Workspace};
use std::collections::BTreeMap;
use std::time::SystemTime;

pub fn run(name: String) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let index_path = ws.forge_dir().join("index");

    // 1. Verify the target branch exists.
    let target_commit = ws.get_branch_tip(&name)?;

    // 2. Bail if already on that branch.
    if let Ok(HeadRef::Branch(current)) = ws.head() {
        if current == name {
            println!("Already on branch '{}'", name);
            return Ok(());
        }
    }

    // 3. Dirty-check: compare index entries against working tree.
    let index = Index::load(&index_path)?;
    for (path, entry) in &index.entries {
        if entry.staged {
            bail!("You have uncommitted changes; commit or stash them first.");
        }

        let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if !abs_path.exists() {
            // File tracked in index but missing on disk — dirty.
            bail!("You have uncommitted changes; commit or stash them first.");
        }

        let metadata = std::fs::metadata(&abs_path)?;
        let mtime = metadata
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();

        if mtime.as_secs() as i64 != entry.mtime_secs
            || mtime.subsec_nanos() != entry.mtime_nanos
            || metadata.len() != entry.size
        {
            // Re-hash to confirm actual content change.
            let data = std::fs::read(&abs_path)?;
            let hash = ForgeHash::from_bytes(&data);
            if hash != entry.hash {
                bail!("You have uncommitted changes; commit or stash them first.");
            }
        }
    }

    // 4. Get current HEAD and target branch trees.
    let current_commit = ws.head_snapshot()?;

    // If target branch points to the same commit, just update HEAD.
    if target_commit == current_commit {
        ws.set_head(&HeadRef::Branch(name.clone()))?;
        println!("Switched to branch '{}'", name);
        return Ok(());
    }

    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();

    let old_flat: BTreeMap<String, (ForgeHash, u64)> = if current_commit.is_zero() {
        BTreeMap::new()
    } else {
        let snap = ws.object_store.get_snapshot(&current_commit)?;
        let tree = ws.object_store.get_tree(&snap.tree)?;
        flatten_tree(&tree, "", &get_tree)
    };

    let new_flat: BTreeMap<String, (ForgeHash, u64)> = if target_commit.is_zero() {
        BTreeMap::new()
    } else {
        let snap = ws.object_store.get_snapshot(&target_commit)?;
        let tree = ws.object_store.get_tree(&snap.tree)?;
        flatten_tree(&tree, "", &get_tree)
    };

    // 5. Diff the two trees.
    let changes = diff_maps(&old_flat, &new_flat);

    // 6. Apply only changed files to working tree.
    for change in &changes {
        match change {
            DiffEntry::Added { path, .. } | DiffEntry::Modified { path, .. } => {
                let (obj_hash, _size) = &new_flat[path];
                let content = read_blob_content(&ws, obj_hash)?;
                let abs = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&abs, &content)?;
            }
            DiffEntry::Deleted { path, .. } => {
                let abs = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
                if abs.exists() {
                    if let Err(e) = std::fs::remove_file(&abs) {
                        eprintln!("warning: could not remove '{}': {}", path, e);
                    }
                }
            }
        }
    }

    // 7. Rebuild full index from target tree.
    let mut new_index = Index::default();
    for (path, (hash, size)) in &new_flat {
        let is_chunked = is_chunked_object(&ws, hash);

        let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let (mtime_secs, mtime_nanos) = if abs_path.exists() {
            mtime_of(&abs_path)
        } else {
            (0, 0)
        };

        let final_content_hash = if is_chunked {
            match read_blob_content(&ws, hash) {
                Ok(data) => ForgeHash::from_bytes(&data),
                Err(_) => ForgeHash::ZERO,
            }
        } else {
            *hash
        };

        new_index.set(
            path.clone(),
            IndexEntry {
                hash: final_content_hash,
                size: *size,
                mtime_secs,
                mtime_nanos,
                staged: false,
                is_chunked,
                object_hash: *hash,
            },
        );
    }
    new_index.save(&index_path)?;

    // 8. Update HEAD to point to the new branch.
    ws.set_head(&HeadRef::Branch(name.clone()))?;

    println!("Switched to branch '{}'", name);
    Ok(())
}

/// Check if an object in the store is a ChunkedBlob (type byte == 2).
fn is_chunked_object(ws: &Workspace, hash: &ForgeHash) -> bool {
    match ws.object_store.chunks.get(hash) {
        Ok(data) if !data.is_empty() => data[0] == 2,
        _ => false,
    }
}

/// Read a blob's content from the object store.
/// For small files, this is the raw blob data.
/// For chunked files, reassemble from the manifest.
fn read_blob_content(ws: &Workspace, object_hash: &ForgeHash) -> Result<Vec<u8>> {
    let data = ws
        .object_store
        .chunks
        .get(object_hash)
        .map_err(|e| anyhow::anyhow!("Failed to read object {}: {}", object_hash.short(), e))?;

    if data.is_empty() {
        bail!("Empty object: {}", object_hash.short());
    }

    if data[0] == 2 {
        // ChunkedBlob manifest — reassemble.
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

/// Get mtime of a file as (secs, nanos).
fn mtime_of(path: &std::path::Path) -> (i64, u32) {
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(mtime) = meta.modified() {
            let dur = mtime
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default();
            return (dur.as_secs() as i64, dur.subsec_nanos());
        }
    }
    (0, 0)
}
