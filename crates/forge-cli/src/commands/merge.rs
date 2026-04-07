use anyhow::{bail, Result};
use chrono::Utc;
use forge_core::diff::flatten_tree;
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::object::snapshot::Snapshot;
use forge_core::object::tree::{EntryKind, Tree, TreeEntry};
use forge_core::store::object_store::ObjectStore;
use forge_core::workspace::{HeadRef, Workspace};
use std::collections::{BTreeMap, HashSet};
use std::time::SystemTime;

pub fn run(branch: String) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    // Get current branch name.
    let current_branch = match ws.head()? {
        HeadRef::Branch(name) => name,
        HeadRef::Detached(_) => bail!("Cannot merge in detached HEAD state."),
    };

    if current_branch == branch {
        bail!("Cannot merge a branch into itself.");
    }

    let ours_hash = ws.head_snapshot()?;
    let theirs_hash = ws.get_branch_tip(&branch)?;

    if theirs_hash.is_zero() {
        bail!("Branch '{}' has no commits.", branch);
    }

    if ours_hash.is_zero() {
        // Current branch has no commits — just fast-forward.
        ws.set_branch_tip(&current_branch, &theirs_hash)?;
        checkout_tree(&ws, &theirs_hash)?;
        println!("Fast-forward merge: {} -> {}", current_branch, theirs_hash.short());
        return Ok(());
    }

    if ours_hash == theirs_hash {
        println!("Already up to date.");
        return Ok(());
    }

    // Check if fast-forward is possible (ours is ancestor of theirs).
    if is_ancestor(&ws.object_store, &ours_hash, &theirs_hash)? {
        ws.set_branch_tip(&current_branch, &theirs_hash)?;
        checkout_tree(&ws, &theirs_hash)?;
        println!("Fast-forward merge: {} -> {}", current_branch, theirs_hash.short());
        return Ok(());
    }

    // Check if theirs is ancestor of ours (already merged).
    if is_ancestor(&ws.object_store, &theirs_hash, &ours_hash)? {
        println!("Already up to date.");
        return Ok(());
    }

    // Three-way merge required.
    let base_hash = find_merge_base(&ws.object_store, &ours_hash, &theirs_hash)?;

    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();

    // Flatten all three trees.
    let base_map = if base_hash.is_zero() {
        BTreeMap::new()
    } else {
        let base_snap = ws.object_store.get_snapshot(&base_hash)?;
        if let Some(tree) = get_tree(&base_snap.tree) {
            flatten_tree(&tree, "", &get_tree)
        } else {
            BTreeMap::new()
        }
    };

    let ours_snap = ws.object_store.get_snapshot(&ours_hash)?;
    let ours_map = if let Some(tree) = get_tree(&ours_snap.tree) {
        flatten_tree(&tree, "", &get_tree)
    } else {
        BTreeMap::new()
    };

    let theirs_snap = ws.object_store.get_snapshot(&theirs_hash)?;
    let theirs_map = if let Some(tree) = get_tree(&theirs_snap.tree) {
        flatten_tree(&tree, "", &get_tree)
    } else {
        BTreeMap::new()
    };

    // Collect all paths.
    let all_paths: HashSet<&String> = base_map
        .keys()
        .chain(ours_map.keys())
        .chain(theirs_map.keys())
        .collect();

    let mut merged: BTreeMap<String, (ForgeHash, u64)> = BTreeMap::new();
    let mut conflicts: Vec<String> = Vec::new();

    for path in &all_paths {
        let base = base_map.get(*path);
        let ours = ours_map.get(*path);
        let theirs = theirs_map.get(*path);

        match (base, ours, theirs) {
            // Both sides same as base — no change.
            (Some(b), Some(o), Some(t)) if o == b && t == b => {
                merged.insert((*path).clone(), *o);
            }
            // Only ours changed.
            (Some(b), Some(o), Some(t)) if t == b && o != b => {
                merged.insert((*path).clone(), *o);
            }
            // Only theirs changed.
            (Some(b), Some(o), Some(t)) if o == b && t != b => {
                merged.insert((*path).clone(), *t);
            }
            // Both changed the same way.
            (Some(_b), Some(o), Some(t)) if o == t => {
                merged.insert((*path).clone(), *o);
            }
            // Both changed differently — conflict.
            (Some(_b), Some(_o), Some(_t)) => {
                conflicts.push((*path).clone());
            }
            // File added only in ours.
            (None, Some(o), None) => {
                merged.insert((*path).clone(), *o);
            }
            // File added only in theirs.
            (None, None, Some(t)) => {
                merged.insert((*path).clone(), *t);
            }
            // Both added same content.
            (None, Some(o), Some(t)) if o == t => {
                merged.insert((*path).clone(), *o);
            }
            // Both added different content — conflict.
            (None, Some(_o), Some(_t)) => {
                conflicts.push((*path).clone());
            }
            // Deleted in ours, unchanged in theirs.
            (Some(b), None, Some(t)) if t == b => {
                // Keep deleted.
            }
            // Deleted in theirs, unchanged in ours.
            (Some(b), Some(o), None) if o == b => {
                // Keep deleted.
            }
            // Deleted in one side, modified in other — conflict.
            (Some(_b), None, Some(_t)) => {
                conflicts.push((*path).clone());
            }
            (Some(_b), Some(_o), None) => {
                conflicts.push((*path).clone());
            }
            // Both deleted.
            (Some(_b), None, None) => {
                // Keep deleted.
            }
            // No entry anywhere.
            (_, None, None) => {}
        }
    }

    if !conflicts.is_empty() {
        conflicts.sort();
        println!("Merge conflict! The following files have conflicting changes:");
        for path in &conflicts {
            println!("  CONFLICT: {}", path);
        }
        bail!(
            "Automatic merge failed. {} conflict(s) found. Resolve manually.",
            conflicts.len()
        );
    }

    // Write merged files to disk and build index.
    let mut index = Index::load(&ws.forge_dir().join("index"))?;

    for (path, (hash, size)) in &merged {
        // Read blob content and write to working directory.
        let content = read_blob(&ws, hash)?;
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
                staged: false,
                is_chunked: false,
                object_hash: *hash,
            },
        );
    }

    // Remove files that were deleted in the merge.
    let merged_paths: HashSet<&String> = merged.keys().collect();
    let index_paths: Vec<String> = index.entries.keys().cloned().collect();
    for path in &index_paths {
        if !merged_paths.contains(path) {
            index.remove(path);
            let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if abs_path.exists() {
                let _ = std::fs::remove_file(&abs_path);
            }
        }
    }

    // Build tree from merged index.
    let all_entries: BTreeMap<String, &IndexEntry> = index
        .entries
        .iter()
        .filter(|(_, v)| !v.hash.is_zero())
        .map(|(k, v)| (k.clone(), v))
        .collect();
    let root_tree = build_tree(&ws, &all_entries)?;
    let tree_hash = ws.object_store.put_tree(&root_tree)?;

    // Create merge commit with two parents.
    let config = ws.config()?;
    let snapshot = Snapshot {
        tree: tree_hash,
        parents: vec![ours_hash, theirs_hash],
        author: config.user.clone(),
        message: format!("Merge branch '{}' into '{}'", branch, current_branch),
        timestamp: Utc::now(),
        metadata: Default::default(),
    };

    let snap_hash = ws.object_store.put_snapshot(&snapshot)?;
    ws.set_branch_tip(&current_branch, &snap_hash)?;

    index.save(&ws.forge_dir().join("index"))?;

    println!("Merged '{}' into '{}' ({})", branch, current_branch, snap_hash.short());

    Ok(())
}

/// Check if `ancestor` is an ancestor of `descendant` by walking the first-parent chain.
fn is_ancestor(store: &ObjectStore, ancestor: &ForgeHash, descendant: &ForgeHash) -> Result<bool> {
    let mut current = *descendant;
    while !current.is_zero() {
        if current == *ancestor {
            return Ok(true);
        }
        let snap = store.get_snapshot(&current)?;
        current = snap.parents.first().copied().unwrap_or(ForgeHash::ZERO);
    }
    Ok(false)
}

/// Find the merge base (common ancestor) of two commits.
fn find_merge_base(store: &ObjectStore, a: &ForgeHash, b: &ForgeHash) -> Result<ForgeHash> {
    // Collect all ancestors of A.
    let mut ancestors_a = HashSet::new();
    let mut cur = *a;
    while !cur.is_zero() {
        ancestors_a.insert(cur);
        let snap = store.get_snapshot(&cur)?;
        cur = snap.parents.first().copied().unwrap_or(ForgeHash::ZERO);
    }
    // Walk B's chain until we find one in A's ancestors.
    cur = *b;
    while !cur.is_zero() {
        if ancestors_a.contains(&cur) {
            return Ok(cur);
        }
        let snap = store.get_snapshot(&cur)?;
        cur = snap.parents.first().copied().unwrap_or(ForgeHash::ZERO);
    }
    Ok(ForgeHash::ZERO)
}

/// Read blob content from the object store (handles both small blobs and chunked blobs).
fn read_blob(ws: &Workspace, hash: &ForgeHash) -> Result<Vec<u8>> {
    let data = ws.object_store.get_blob_data(hash)?;
    Ok(data)
}

/// Update working tree and index to match a given snapshot.
fn checkout_tree(ws: &Workspace, snap_hash: &ForgeHash) -> Result<()> {
    let snap = ws.object_store.get_snapshot(snap_hash)?;
    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();
    let tree = ws
        .object_store
        .get_tree(&snap.tree)
        .map_err(|e| anyhow::anyhow!("Failed to get root tree: {}", e))?;
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

    // Write all files from the target tree.
    for (path, (hash, size)) in &file_map {
        let content = read_blob(ws, hash)?;
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
                staged: false,
                is_chunked: false,
                object_hash: *hash,
            },
        );
    }

    index.save(&ws.forge_dir().join("index"))?;
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
