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

    let target_hash = ForgeHash::from_hex(&commit)?;
    let target_snap = ws.object_store.get_snapshot(&target_hash)?;

    let parent_hash = target_snap
        .parents
        .first()
        .copied()
        .unwrap_or(ForgeHash::ZERO);

    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();

    // Get parent tree (empty if initial commit).
    let parent_map = if parent_hash.is_zero() {
        BTreeMap::new()
    } else {
        let parent_snap = ws.object_store.get_snapshot(&parent_hash)?;
        if let Some(tree) = get_tree(&parent_snap.tree) {
            flatten_tree(&tree, "", &get_tree)
        } else {
            BTreeMap::new()
        }
    };

    // Get target tree.
    let target_map = if let Some(tree) = get_tree(&target_snap.tree) {
        flatten_tree(&tree, "", &get_tree)
    } else {
        BTreeMap::new()
    };

    // Diff parent vs target to get the changes introduced by this commit.
    let changes = diff_maps(&parent_map, &target_map);

    if changes.is_empty() {
        bail!("Commit {} introduces no changes.", target_hash.short());
    }

    let mut index = Index::load(&ws.forge_dir().join("index"))?;

    // Apply changes to working directory.
    for change in &changes {
        match change {
            DiffEntry::Added { path, hash, size } | DiffEntry::Modified { path, new_hash: hash, new_size: size, .. } => {
                let content = ws.object_store.get_blob_data(hash)?;
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
                    path.clone(),
                    IndexEntry {
                        hash: ForgeHash::from_bytes(&content),
                        size: *size,
                        mtime_secs: mtime.as_secs() as i64,
                        mtime_nanos: mtime.subsec_nanos(),
                        staged: true,
                        is_chunked: false,
                        object_hash: *hash,
                    },
                );
            }
            DiffEntry::Deleted { path, .. } => {
                let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
                if abs_path.exists() {
                    std::fs::remove_file(&abs_path)?;
                }
                if let Some(entry) = index.entries.get_mut(path) {
                    entry.staged = true;
                    entry.hash = ForgeHash::ZERO;
                    entry.object_hash = ForgeHash::ZERO;
                    entry.size = 0;
                }
            }
        }
    }

    // Build tree from current index (excluding zero-hash deletions).
    let all_entries: BTreeMap<String, &IndexEntry> = index
        .entries
        .iter()
        .filter(|(_, v)| !v.hash.is_zero())
        .map(|(k, v)| (k.clone(), v))
        .collect();
    let root_tree = build_tree(&ws, &all_entries)?;
    let tree_hash = ws.object_store.put_tree(&root_tree)?;

    // Create new commit.
    let head_hash = ws.head_snapshot()?;
    let parents = if head_hash.is_zero() {
        vec![]
    } else {
        vec![head_hash]
    };

    let config = ws.config()?;
    let snapshot = Snapshot {
        tree: tree_hash,
        parents,
        author: config.user.clone(),
        message: format!("cherry-pick: {}", target_snap.message),
        timestamp: Utc::now(),
        metadata: Default::default(),
    };

    let snap_hash = ws.object_store.put_snapshot(&snapshot)?;

    // Update branch ref.
    if let HeadRef::Branch(branch) = ws.head()? {
        ws.set_branch_tip(&branch, &snap_hash)?;
    }

    // Clean up deleted entries and clear staged flags.
    index.entries.retain(|_, e| !e.hash.is_zero());
    index.clear_staged();
    index.save(&ws.forge_dir().join("index"))?;

    println!(
        "Cherry-picked {} -> {}",
        target_hash.short(),
        snap_hash.short()
    );
    for change in &changes {
        match change {
            DiffEntry::Added { path, .. } => println!("  \x1b[32mA\x1b[0m  {}", path),
            DiffEntry::Deleted { path, .. } => println!("  \x1b[31mD\x1b[0m  {}", path),
            DiffEntry::Modified { path, .. } => println!("  \x1b[33mM\x1b[0m  {}", path),
        }
    }

    Ok(())
}

/// Build a Tree hierarchy from index entries (same logic as snapshot.rs).
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
