use anyhow::Result;
use forge_core::hash::ForgeHash;
use forge_core::index::Index;
use forge_core::workspace::Workspace;
use std::path::Path;

pub fn run(paths: Vec<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let mut index = Index::load(&ws.forge_dir().join("index"))?;

    // Build set of paths from previous commit to restore hash on unstage.
    let head = ws.head_snapshot()?;
    let prev_tree = if !head.is_zero() {
        Some(ws.object_store.get_snapshot(&head)?)
    } else {
        None
    };

    let unstage_all = paths.iter().any(|p| p == ".");

    // Collect matching entries.
    let keys: Vec<String> = index.entries.keys().cloned().collect();
    let mut count = 0usize;

    for key in &keys {
        let dominated = if unstage_all {
            true
        } else {
            paths.iter().any(|p| {
                let norm = p.replace('\\', "/");
                let norm_path = if Path::new(&norm).is_absolute() {
                    norm.strip_prefix(
                        &ws.root.to_string_lossy().replace('\\', "/")
                    )
                    .unwrap_or(&norm)
                    .trim_start_matches('/')
                    .to_string()
                } else {
                    norm
                };
                key == &norm_path || key.starts_with(&format!("{}/", norm_path))
            })
        };

        if !dominated {
            continue;
        }

        let entry = match index.entries.get(key) {
            Some(e) if e.staged => e,
            _ => continue,
        };

        if entry.hash.is_zero() {
            // Was a staged deletion — need to restore the entry from the previous commit.
            // For now, remove the ZERO-hash entry; status will show it as deleted (unstaged).
            // We need to restore the original hash from the last commit tree.
            if let Some(ref snap) = prev_tree {
                if let Some(original) = find_entry_in_tree(&ws, &snap.tree, key) {
                    let e = index.entries.get_mut(key).expect("key exists in index");
                    e.staged = false;
                    e.hash = original.0;
                    e.object_hash = original.1;
                    e.size = original.2;
                } else {
                    // File didn't exist in previous commit — just remove it.
                    index.entries.remove(key);
                }
            } else {
                index.entries.remove(key);
            }
        } else {
            if let Some(e) = index.entries.get_mut(key) {
                e.staged = false;
            }
        }
        count += 1;
    }

    if count > 0 {
        index.save(&ws.forge_dir().join("index"))?;
    }

    Ok(())
}

/// Find a file entry in the commit tree by path, returns (content_hash, object_hash, size).
fn find_entry_in_tree(
    ws: &Workspace,
    tree_hash: &ForgeHash,
    path: &str,
) -> Option<(ForgeHash, ForgeHash, u64)> {
    use forge_core::object::tree::EntryKind;

    let tree = ws.object_store.get_tree(tree_hash).ok()?;

    if let Some(slash) = path.find('/') {
        let dir = &path[..slash];
        let rest = &path[slash + 1..];
        for entry in &tree.entries {
            if entry.name == dir && entry.kind == EntryKind::Directory {
                return find_entry_in_tree(ws, &entry.hash, rest);
            }
        }
    } else {
        for entry in &tree.entries {
            if entry.name == path && entry.kind == EntryKind::File {
                return Some((entry.hash, entry.hash, entry.size));
            }
        }
    }
    None
}
