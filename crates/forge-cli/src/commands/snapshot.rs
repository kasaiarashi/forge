use anyhow::{bail, Result};
use chrono::Utc;
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::object::snapshot::Snapshot;
use forge_core::object::tree::{EntryKind, Tree, TreeEntry};
use forge_core::workspace::{HeadRef, Workspace};
use std::collections::BTreeMap;

pub fn run(message: String, all: bool, json: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let mut index = Index::load(&ws.forge_dir().join("index"))?;

    // If --all, auto-stage all modified/deleted files.
    if all {
        auto_stage(&ws, &mut index)?;
    }

    let staged: Vec<(String, forge_core::index::IndexEntry)> = index
        .entries
        .iter()
        .filter(|(_, e)| e.staged)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    if staged.is_empty() {
        bail!("Nothing staged. Use `forge add` or `forge commit --all`.");
    }

    // Build tree hierarchy from all entries, excluding staged deletions (ZERO hash).
    let all_entries: BTreeMap<String, &IndexEntry> = index
        .entries
        .iter()
        .filter(|(_, v)| !v.hash.is_zero())
        .map(|(k, v)| (k.clone(), v))
        .collect();
    let root_tree = build_tree(&ws, &all_entries)?;
    let tree_hash = ws.object_store.put_tree(&root_tree)?;

    // Get parent snapshot.
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
        message: message.clone(),
        timestamp: Utc::now(),
        metadata: Default::default(),
    };

    let snap_hash = ws.object_store.put_snapshot(&snapshot)?;

    // Update branch ref.
    if let HeadRef::Branch(branch) = ws.head()? {
        ws.set_branch_tip(&branch, &snap_hash)?;
    }

    // Remove deleted entries (ZERO hash) and clear staged flags.
    index.entries.retain(|_, e| !e.hash.is_zero());
    index.clear_staged();
    index.save(&ws.forge_dir().join("index"))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "hash": snap_hash.to_hex(),
                "short_hash": snap_hash.short(),
                "message": message,
                "files": staged.len(),
            }))?
        );
    } else {
        println!("Committed {}", snap_hash.short());
        println!("  {} file(s) | {}", staged.len(), message);
    }

    Ok(())
}

fn auto_stage(ws: &Workspace, index: &mut Index) -> Result<()> {
    let ignore = forge_ignore::ForgeIgnore::from_file(&ws.root.join(".forgeignore"))
        .unwrap_or_else(|_| forge_ignore::ForgeIgnore::from_str("").unwrap());

    // Check existing entries for modifications.
    let paths: Vec<String> = index.entries.keys().cloned().collect();
    for path in &paths {
        let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if !abs_path.exists() {
            if let Some(entry) = index.entries.get_mut(path) {
                entry.staged = true;
            }
            continue;
        }

        let data = std::fs::read(&abs_path)?;
        let hash = ForgeHash::from_bytes(&data);
        if let Some(entry) = index.entries.get(path) {
            if hash != entry.hash {
                // File modified — re-add it.
                crate::commands::add::run(vec![path.clone()])?;
            }
        }
    }

    // Also add untracked files.
    for entry in walkdir::WalkDir::new(&ws.root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(&ws.root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");

            if rel.starts_with(".forge/") || rel.starts_with(".forge\\") {
                continue;
            }
            if ignore.is_ignored(&rel) {
                continue;
            }
            if !index.entries.contains_key(&rel) {
                crate::commands::add::run(vec![rel])?;
                // Reload index since add modifies it.
                *index = Index::load(&ws.forge_dir().join("index"))?;
            }
        }
    }

    Ok(())
}

/// Build a Tree hierarchy from all index entries.
fn build_tree(
    ws: &Workspace,
    entries: &BTreeMap<String, &IndexEntry>,
) -> Result<Tree> {
    // Group entries by top-level directory component.
    let mut dirs: BTreeMap<String, BTreeMap<String, &IndexEntry>> =
        BTreeMap::new();
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

    // Recursively build subtrees for directories.
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
