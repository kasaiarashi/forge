use anyhow::Result;
use forge_core::hash::ForgeHash;
use forge_core::index::Index;
use forge_core::object::tree::EntryKind;
use forge_core::workspace::Workspace;
use rayon::prelude::*;
use std::time::SystemTime;

/// Recursively collect all file paths from a tree into a set.
fn collect_tree_paths(
    ws: &Workspace,
    tree_hash: &ForgeHash,
    prefix: &str,
    out: &mut std::collections::HashSet<String>,
) {
    let tree = match ws.object_store.get_tree(tree_hash) {
        Ok(t) => t,
        Err(_) => return,
    };
    for entry in &tree.entries {
        let full = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };
        match entry.kind {
            EntryKind::Directory => collect_tree_paths(ws, &entry.hash, &full, out),
            _ => { out.insert(full); }
        }
    }
}

/// Fetch active locks from the server (best-effort with 1s timeout, returns empty on failure).
fn fetch_locks(ws: &Workspace) -> Vec<(String, String)> {
    let config = match ws.config() {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let server_url = match config.default_remote_url() {
        Some(u) => u.to_string(),
        None => return vec![],
    };

    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    rt.block_on(async {
        use forge_proto::forge::*;

        // Timeout the entire lock fetch to avoid slowing down status.
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            let mut client = crate::client::connect_forge(&server_url).await?;

            let repo = if config.repo.is_empty() {
                "default".into()
            } else {
                config.repo.clone()
            };

            let resp = client
                .list_locks(ListLocksRequest {
                    repo,
                    path_prefix: String::new(),
                    owner: String::new(),
                })
                .await?
                .into_inner();

            Ok::<Vec<(String, String)>, Box<dyn std::error::Error>>(
                resp.locks.into_iter().map(|l| (l.path, l.owner)).collect(),
            )
        })
        .await;

        match result {
            Ok(Ok(locks)) => locks,
            _ => vec![],
        }
    })
}

pub fn run(json: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let index = Index::load(&ws.forge_dir().join("index"))?;
    let ignore = forge_ignore::ForgeIgnore::from_file(&ws.root.join(".forgeignore"))
        .unwrap_or_default();

    // Staged entries, sub-categorized like git status.
    let mut staged_new = Vec::new();
    let mut staged_modified = Vec::new();
    let mut staged_deleted = Vec::new();
    // Unstaged changes.
    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    let mut untracked = Vec::new();

    // Check if there are any staged entries first (optimization: skip tree walk if none).
    let has_any_staged = index.entries.values().any(|e| e.staged);

    // Only build prev_paths if we have staged entries (avoids expensive tree traversal).
    let prev_paths = if has_any_staged {
        let mut set = std::collections::HashSet::new();
        let head = ws.head_snapshot()?;
        if !head.is_zero() {
            if let Ok(snap) = ws.object_store.get_snapshot(&head) {
                collect_tree_paths(&ws, &snap.tree, "", &mut set);
            }
        }
        set
    } else {
        std::collections::HashSet::new()
    };

    // Process staged entries first (small set, sequential is fine).
    let seen: std::collections::HashSet<String> = index.entries.keys().cloned().collect();
    for (path, entry) in &index.entries {
        if !entry.staged {
            continue;
        }
        let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let exists = abs_path.exists();
        if !exists && entry.hash == ForgeHash::ZERO {
            staged_deleted.push(path.clone());
        } else if !exists {
            deleted.push(path.clone());
        } else if prev_paths.contains(path.as_str()) {
            staged_modified.push(path.clone());
        } else {
            staged_new.push(path.clone());
        }
    }

    // Check unstaged index entries against working tree — parallel metadata + hash checks.
    let ws_root = &ws.root;
    let unstaged_results: Vec<(String, &str)> = index
        .entries
        .par_iter()
        .filter(|(_, entry)| !entry.staged)
        .filter_map(|(path, entry)| {
            let abs_path = ws_root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if !abs_path.exists() {
                return Some((path.clone(), "deleted"));
            }
            let metadata = match std::fs::metadata(&abs_path) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("warning: cannot stat '{}': {}", path, e);
                    return Some((path.clone(), "modified"));
                }
            };
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
                .unwrap_or_default();
            if mtime.as_secs() as i64 == entry.mtime_secs
                && mtime.subsec_nanos() == entry.mtime_nanos
                && metadata.len() == entry.size
            {
                return None; // unchanged
            }
            // Re-hash to confirm.
            let data = match std::fs::read(&abs_path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("warning: cannot read '{}': {}", path, e);
                    return Some((path.clone(), "modified"));
                }
            };
            let hash = ForgeHash::from_bytes(&data);
            if hash != entry.hash {
                Some((path.clone(), "modified"))
            } else {
                None
            }
        })
        .collect();

    for (path, status) in unstaged_results {
        match status {
            "deleted" => deleted.push(path),
            "modified" => modified.push(path),
            _ => {}
        }
    }

    // Find untracked files. Use filter_entry to skip .forge/ and ignored directories entirely.
    let forge_dir_name = std::ffi::OsStr::new(".forge");
    for entry in walkdir::WalkDir::new(&ws.root)
        .into_iter()
        .filter_entry(|e| {
            // Skip .forge directory and common ignored directories at the entry level
            // so walkdir doesn't descend into them at all.
            let name = e.file_name();
            if name == forge_dir_name {
                return false;
            }
            if e.file_type().is_dir() {
                let rel = e
                    .path()
                    .strip_prefix(&ws.root)
                    .unwrap_or(e.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                if !rel.is_empty() && ignore.is_ignored(&rel) {
                    return false; // Skip entire ignored directory tree
                }
            }
            true
        })
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(&ws.root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");

            if ignore.is_ignored(&rel) {
                continue;
            }

            if !seen.contains(&rel) {
                untracked.push(rel);
            }
        }
    }

    // Fetch locks from server (best-effort).
    let locks = fetch_locks(&ws);

    let has_staged = !staged_new.is_empty() || !staged_modified.is_empty() || !staged_deleted.is_empty();

    if json {
        let lock_entries: Vec<serde_json::Value> = locks
            .iter()
            .map(|(p, o)| serde_json::json!({"path": p, "owner": o}))
            .collect();
        let output = serde_json::json!({
            "staged_new": staged_new,
            "staged_modified": staged_modified,
            "staged_deleted": staged_deleted,
            "modified": modified,
            "deleted": deleted,
            "untracked": untracked,
            "locked": lock_entries,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if let Some(branch) = ws.current_branch()? {
            println!("On branch {}", branch);
        }
        println!();

        if !locks.is_empty() {
            println!("Locked files:");
            for (path, owner) in &locks {
                println!("  \x1b[35m  locked: {} (by {})\x1b[0m", path, owner);
            }
            println!();
        }

        if has_staged {
            println!("Changes to be committed:");
            println!("  (use \"forge unstage <file>...\" to unstage)");
            println!();
            for f in &staged_new {
                println!("        \x1b[32mnew file:   {}\x1b[0m", f);
            }
            for f in &staged_modified {
                println!("        \x1b[32mmodified:   {}\x1b[0m", f);
            }
            for f in &staged_deleted {
                println!("        \x1b[32mdeleted:    {}\x1b[0m", f);
            }
            println!();
        }

        if !modified.is_empty() || !deleted.is_empty() {
            println!("Changes not staged for commit:");
            println!("  (use \"forge add <file>...\" to update what will be committed)");
            println!("  (use \"forge restore <file>...\" to discard changes in working directory)");
            println!();
            for f in &modified {
                println!("        \x1b[31mmodified:   {}\x1b[0m", f);
            }
            for f in &deleted {
                println!("        \x1b[31mdeleted:    {}\x1b[0m", f);
            }
            println!();
        }

        if !untracked.is_empty() {
            println!("Untracked files:");
            println!("  (use \"forge add <file>...\" to include in what will be committed)");
            println!();
            for f in &untracked {
                println!("        \x1b[31m{}\x1b[0m", f);
            }
            println!();
        }

        if !has_staged && modified.is_empty() && deleted.is_empty() && untracked.is_empty() && locks.is_empty() {
            println!("Nothing to report — working tree clean.");
        }
    }

    Ok(())
}
