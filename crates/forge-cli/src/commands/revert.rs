use anyhow::{bail, Result};
use chrono::Utc;
use forge_core::diff::{diff_maps, flatten_tree, DiffEntry};
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::object::snapshot::Snapshot;
use forge_core::object::tree::{EntryKind, Tree, TreeEntry};
use forge_core::workspace::{HeadRef, Workspace};
use std::collections::BTreeMap;
use std::time::SystemTime;

pub fn run(commit: String) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let index_path = ws.forge_dir().join("index");

    let target_hash = ForgeHash::from_hex(&commit)
        .map_err(|_| anyhow::anyhow!("Invalid commit hash: {}", commit))?;

    let target_snap = ws.object_store.get_snapshot(&target_hash)
        .map_err(|_| anyhow::anyhow!("Not a valid commit: {}", commit))?;

    // Get parent snapshot (first parent). If no parent, use empty tree.
    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();

    let parent_flat = if target_snap.parents.is_empty() {
        BTreeMap::new()
    } else {
        let parent_snap = ws.object_store.get_snapshot(&target_snap.parents[0])?;
        let parent_tree = ws.object_store.get_tree(&parent_snap.tree)?;
        flatten_tree(&parent_tree, "", &get_tree)
    };

    let target_tree = ws.object_store.get_tree(&target_snap.tree)?;
    let target_flat = flatten_tree(&target_tree, "", &get_tree);

    // Diff parent vs target to find what the commit changed.
    let changes = diff_maps(&parent_flat, &target_flat);

    if changes.is_empty() {
        bail!("Commit {} introduced no changes.", target_hash.short());
    }

    let mut index = Index::load(&index_path)?;

    for change in &changes {
        match change {
            DiffEntry::Added { path, .. } => {
                // The commit added this file. To revert, delete it.
                let abs = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
                if abs.exists() {
                    std::fs::remove_file(&abs)?;
                }
                // Stage deletion in the index.
                if let Some(entry) = index.entries.get_mut(path) {
                    entry.hash = ForgeHash::ZERO;
                    entry.object_hash = ForgeHash::ZERO;
                    entry.size = 0;
                    entry.staged = true;
                } else {
                    // Not in index — add a zero-hash entry to stage deletion.
                    index.set(path.clone(), IndexEntry {
                        hash: ForgeHash::ZERO,
                        object_hash: ForgeHash::ZERO,
                        size: 0,
                        mtime_secs: 0,
                        mtime_nanos: 0,
                        staged: true,
                        is_chunked: false,
                    });
                }
            }
            DiffEntry::Deleted { path, hash, size } => {
                // The commit deleted this file. To revert, restore from parent.
                restore_file(&ws, path, hash, *size, &mut index)?;
            }
            DiffEntry::Modified { path, old_hash, old_size, .. } => {
                // The commit modified this file. To revert, restore old version from parent.
                restore_file(&ws, path, old_hash, *old_size, &mut index)?;
            }
        }
    }

    index.save(&index_path)?;

    // Now create a revert commit using the same pattern as snapshot.rs.
    // Reload index to build tree from current state.
    let index = Index::load(&index_path)?;

    let all_entries: BTreeMap<String, &IndexEntry> = index
        .entries
        .iter()
        .filter(|(_, v)| !v.hash.is_zero())
        .map(|(k, v)| (k.clone(), v))
        .collect();

    let root_tree = build_tree(&ws, &all_entries)?;
    let tree_hash = ws.object_store.put_tree(&root_tree)?;

    let head_hash = ws.head_snapshot()?;
    let parents = if head_hash.is_zero() {
        vec![]
    } else {
        vec![head_hash]
    };

    let config = ws.config()?;
    let revert_message = format!("Revert \"{}\"", target_snap.message);
    let snapshot = Snapshot {
        tree: tree_hash,
        parents,
        author: config.user.clone(),
        message: revert_message.clone(),
        timestamp: Utc::now(),
        metadata: Default::default(),
    };

    let snap_hash = ws.object_store.put_snapshot(&snapshot)?;

    // Update branch ref.
    if let HeadRef::Branch(branch) = ws.head()? {
        ws.set_branch_tip(&branch, &snap_hash)?;
    }

    // Clear staged flags and remove zero-hash entries.
    let mut index = Index::load(&index_path)?;
    index.entries.retain(|_, e| !e.hash.is_zero());
    index.clear_staged();
    index.save(&index_path)?;

    println!("Committed {} — {}", snap_hash.short(), revert_message);
    Ok(())
}

/// Restore a file from the object store to disk and stage it in the index.
fn restore_file(
    ws: &Workspace,
    path: &str,
    object_hash: &ForgeHash,
    size: u64,
    index: &mut Index,
) -> Result<()> {
    let content = read_blob_content(ws, object_hash)?;
    let abs = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&abs, &content)?;

    let mtime = std::fs::metadata(&abs)?
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    let content_hash = ForgeHash::from_bytes(&content);

    index.set(path.to_string(), IndexEntry {
        hash: content_hash,
        size,
        mtime_secs: mtime.as_secs() as i64,
        mtime_nanos: mtime.subsec_nanos(),
        staged: true,
        is_chunked: false,
        object_hash: *object_hash,
    });

    Ok(())
}

/// Read blob content, handling both small and chunked blobs.
fn read_blob_content(ws: &Workspace, object_hash: &ForgeHash) -> Result<Vec<u8>> {
    let data = ws.object_store.chunks.get(object_hash)
        .map_err(|e| anyhow::anyhow!("Failed to read object {}: {}", object_hash.short(), e))?;

    if data.is_empty() {
        anyhow::bail!("Empty object: {}", object_hash.short());
    }

    if data[0] == 2 {
        // ChunkedBlob manifest.
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

/// Build a tree hierarchy from index entries (same logic as snapshot.rs).
fn build_tree(
    ws: &Workspace,
    entries: &BTreeMap<String, &IndexEntry>,
) -> Result<Tree> {
    let mut dirs: BTreeMap<String, BTreeMap<String, &IndexEntry>> = BTreeMap::new();
    let mut files: Vec<TreeEntry> = Vec::new();

    for (path, entry) in entries {
        if let Some(slash_pos) = path.find('/') {
            let dir_name = &path[..slash_pos];
            let rest = &path[slash_pos + 1..];
            dirs.entry(dir_name.to_string())
                .or_default()
                .insert(rest.to_string(), entry);
        } else {
            files.push(TreeEntry {
                name: path.clone(),
                kind: EntryKind::File,
                hash: entry.object_hash,
                size: entry.size,
            });
        }
    }

    for (dir_name, sub_entries) in &dirs {
        let subtree = build_tree(ws, sub_entries)?;
        let subtree_hash = ws.object_store.put_tree(&subtree)?;
        files.push(TreeEntry {
            name: dir_name.clone(),
            kind: EntryKind::Directory,
            hash: subtree_hash,
            size: 0,
        });
    }

    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Tree { entries: files })
}
