use anyhow::{bail, Result};
use forge_core::diff::flatten_tree;
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::workspace::Workspace;
use std::time::SystemTime;

pub fn run(target: Option<String>, paths: Vec<String>) -> Result<()> {
    if paths.is_empty() {
        // No paths: switch branches (delegate to switch).
        match target {
            Some(name) => return crate::commands::switch::run(name),
            None => bail!("Usage: forge checkout <branch> or forge checkout [<commit>] -- <paths>"),
        }
    }

    // Paths given: restore files from a commit (or HEAD).
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    let commit_hash = match &target {
        Some(ref_str) => {
            // Try as branch name first, then as hex hash.
            ws.get_branch_tip(ref_str)
                .or_else(|_| ForgeHash::from_hex(ref_str))?
        }
        None => ws.head_snapshot()?,
    };

    if commit_hash.is_zero() {
        bail!("No commits to restore from.");
    }

    let snap = ws.object_store.get_snapshot(&commit_hash)?;
    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();
    let tree = ws.object_store.get_tree(&snap.tree)?;
    let file_map = flatten_tree(&tree, "", &get_tree);

    let mut index = Index::load(&ws.forge_dir().join("index"))?;
    let mut restored = 0usize;

    for path in &paths {
        // Normalize path separators.
        let normalized = path.replace('\\', "/");

        // Check for exact match or prefix match (directory restore).
        let matching: Vec<(String, ForgeHash, u64)> = file_map
            .iter()
            .filter(|(p, _)| {
                *p == &normalized || p.starts_with(&format!("{}/", normalized))
            })
            .map(|(p, (h, s))| (p.clone(), *h, *s))
            .collect();

        if matching.is_empty() {
            eprintln!("warning: path '{}' not found in commit {}", path, commit_hash.short());
            continue;
        }

        for (file_path, hash, size) in matching {
            let content = ws.object_store.get_blob_data(&hash)?;
            let abs_path = ws.root.join(file_path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if let Some(parent) = abs_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&abs_path, &content)?;

            let mtime = std::fs::metadata(&abs_path)?
                .modified()?
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default();

            index.set(
                file_path.clone(),
                IndexEntry {
                    hash: ForgeHash::from_bytes(&content),
                    size,
                    mtime_secs: mtime.as_secs() as i64,
                    mtime_nanos: mtime.subsec_nanos(),
                    staged: false,
                    is_chunked: false,
                    object_hash: hash,
                },
            );

            restored += 1;
            println!("Restored '{}'", file_path);
        }
    }

    index.save(&ws.forge_dir().join("index"))?;

    if restored == 0 {
        bail!("No files restored.");
    }

    println!("Restored {} file(s) from {}", restored, commit_hash.short());
    Ok(())
}
