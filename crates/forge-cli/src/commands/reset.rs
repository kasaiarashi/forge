use anyhow::{bail, Result};
use forge_core::diff::flatten_tree;
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::workspace::{HeadRef, Workspace};
use std::collections::BTreeMap;

pub fn run(commit: Option<String>, soft: bool, hard: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let index_path = ws.forge_dir().join("index");

    // No commit argument: just unstage everything.
    if commit.is_none() && !soft && !hard {
        let mut index = Index::load(&index_path)?;
        index.clear_staged();
        index.save(&index_path)?;
        println!("Unstaged all changes.");
        return Ok(());
    }

    // Resolve target commit.
    let target_hex = commit.unwrap_or_else(|| String::new());
    if target_hex.is_empty() {
        bail!("A commit hash is required for --soft or --hard reset.");
    }

    let target_hash = ForgeHash::from_hex(&target_hex)
        .map_err(|_| anyhow::anyhow!("Invalid commit hash: {}", target_hex))?;

    // Verify the target is a valid snapshot.
    let _target_snap = ws.object_store.get_snapshot(&target_hash)
        .map_err(|_| anyhow::anyhow!("Not a valid commit: {}", target_hex))?;

    // Move HEAD (branch tip) to the target commit.
    match ws.head()? {
        HeadRef::Branch(branch) => {
            ws.set_branch_tip(&branch, &target_hash)?;
        }
        HeadRef::Detached(_) => {
            ws.set_head(&HeadRef::Detached(target_hash))?;
        }
    }

    if soft {
        println!("Soft reset to {}", target_hash.short());
        return Ok(());
    }

    // Mixed (default) or hard: rebuild index from target tree.
    let target_snap = ws.object_store.get_snapshot(&target_hash)?;
    let target_tree = ws.object_store.get_tree(&target_snap.tree)?;

    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();
    let flat = flatten_tree(&target_tree, "", &get_tree);

    let mut index = Index::default();
    for (path, (hash, size)) in &flat {
        // Determine if it's a chunked blob by checking the object type byte.
        let is_chunked = is_chunked_object(&ws, hash);

        // For the index entry, use the file on disk if it exists for mtime.
        let rel_disk = path.replace('/', std::path::MAIN_SEPARATOR_STR);
        let abs_path = ws.root.join(&rel_disk);
        let (mtime_secs, mtime_nanos) = if abs_path.exists() {
            mtime_of(&abs_path)
        } else {
            (0, 0)
        };

        // For non-chunked files, content_hash == object_hash == tree entry hash.
        // For chunked, we set hash = ZERO (unknown content hash) — but that breaks
        // status checks. Better: read the blob to get its actual content hash.
        let final_content_hash = if is_chunked {
            // Try to reassemble and hash the content.
            match read_blob_content(&ws, hash) {
                Ok(data) => ForgeHash::from_bytes(&data),
                Err(_) => ForgeHash::ZERO,
            }
        } else {
            // Small file: blob data IS the content. The tree stores
            // object_hash which for small files is the content hash.
            *hash
        };

        index.set(path.clone(), IndexEntry {
            hash: final_content_hash,
            size: *size,
            mtime_secs,
            mtime_nanos,
            staged: false,
            is_chunked,
            object_hash: *hash,
        });
    }
    index.save(&index_path)?;

    if hard {
        // Hard reset: restore working tree to match the target tree.
        restore_working_tree(&ws, &flat)?;
        println!("Hard reset to {}", target_hash.short());
    } else {
        println!("Reset to {}", target_hash.short());
    }

    Ok(())
}

/// Check if an object in the store is a ChunkedBlob (type byte == 2).
fn is_chunked_object(ws: &Workspace, hash: &ForgeHash) -> bool {
    match ws.object_store.chunks.get(hash) {
        Ok(data) if !data.is_empty() => data[0] == 2, // ObjectType::ChunkedBlob
        _ => false,
    }
}

/// Read a blob's content from the object store.
/// For small files, this is the raw blob data.
/// For chunked files, reassemble from the manifest.
fn read_blob_content(ws: &Workspace, object_hash: &ForgeHash) -> Result<Vec<u8>> {
    let data = ws.object_store.chunks.get(object_hash)
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
        // Raw blob data — the object store stores type-prefixed data for typed objects,
        // but for raw blobs (put_blob_data) there is no type prefix.
        // Small files stored via put_blob_data have no prefix.
        // Actually, get_blob_data just calls chunks.get which decompresses.
        // The data here is the raw decompressed content from chunk store.
        // But wait — for small files, add.rs stores the raw file data directly
        // (WholeFile branch), not type-prefixed. So the data IS the file content.
        Ok(data)
    }
}

/// Get mtime of a file as (secs, nanos).
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

/// Restore working tree files from a flattened tree map.
fn restore_working_tree(
    ws: &Workspace,
    flat: &BTreeMap<String, (ForgeHash, u64)>,
) -> Result<()> {
    // Collect existing tracked files.
    let index_path = ws.forge_dir().join("index");
    let old_index = Index::load(&index_path).unwrap_or_default();

    // Delete files not in the target tree.
    for (path, _) in &old_index.entries {
        if !flat.contains_key(path) {
            let abs = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if abs.exists() {
                let _ = std::fs::remove_file(&abs);
            }
        }
    }

    // Write all files from the target tree.
    for (path, (hash, _size)) in flat {
        let content = read_blob_content(ws, hash)?;
        let abs = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&abs, &content)?;
    }

    // Rebuild index with correct mtimes after writing.
    let mut index = Index::load(&index_path)?;
    for (path, entry) in index.entries.iter_mut() {
        let abs = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let (secs, nanos) = mtime_of(&abs);
        entry.mtime_secs = secs;
        entry.mtime_nanos = nanos;
    }
    index.save(&index_path)?;

    Ok(())
}
