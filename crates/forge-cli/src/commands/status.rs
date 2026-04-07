use anyhow::Result;
use forge_core::hash::ForgeHash;
use forge_core::index::Index;
use forge_core::object::tree::EntryKind;
use forge_core::workspace::Workspace;
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
        use forge_proto::forge::forge_service_client::ForgeServiceClient;
        use forge_proto::forge::*;

        // Timeout the entire lock fetch to avoid slowing down status.
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            let mut client = ForgeServiceClient::connect(server_url).await?;

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
        .unwrap_or_else(|_| forge_ignore::ForgeIgnore::from_str("").unwrap());

    // Staged entries, sub-categorized like git status.
    let mut staged_new = Vec::new();
    let mut staged_modified = Vec::new();
    let mut staged_deleted = Vec::new();
    // Unstaged changes.
    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    let mut untracked = Vec::new();

    // Build set of paths from previous commit to distinguish new vs modified.
    let prev_paths = {
        let mut set = std::collections::HashSet::new();
        let head = ws.head_snapshot()?;
        if !head.is_zero() {
            if let Ok(snap) = ws.object_store.get_snapshot(&head) {
                collect_tree_paths(&ws, &snap.tree, "", &mut set);
            }
        }
        set
    };

    // Check all index entries against working tree.
    let mut seen = std::collections::HashSet::new();
    for (path, entry) in &index.entries {
        seen.insert(path.clone());
        let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let exists = abs_path.exists();

        if entry.staged {
            if !exists && entry.hash == ForgeHash::ZERO {
                // Intentionally staged deletion (via forge add after delete).
                staged_deleted.push(path.clone());
                continue;
            } else if !exists {
                // File was staged (e.g., modified) but then deleted without re-staging.
                // Staging is stale — show as unstaged deletion.
                deleted.push(path.clone());
                continue;
            } else {
                // File exists and is staged.
                if prev_paths.contains(path.as_str()) {
                    staged_modified.push(path.clone());
                } else {
                    staged_new.push(path.clone());
                }
                continue;
            }
        }

        if !exists {
            deleted.push(path.clone());
            continue;
        }

        // Fast path: check mtime + size.
        let metadata = std::fs::metadata(&abs_path)?;
        let mtime = metadata
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();

        if mtime.as_secs() as i64 != entry.mtime_secs
            || mtime.subsec_nanos() != entry.mtime_nanos
            || metadata.len() != entry.size
        {
            // Re-hash to confirm.
            let data = std::fs::read(&abs_path)?;
            let hash = ForgeHash::from_bytes(&data);
            if hash != entry.hash {
                modified.push(path.clone());
            }
            // else: mtime changed but content same — could update index mtime
        }
    }

    // Find untracked files.
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

            // Skip .forge directory.
            if rel.starts_with(".forge/") || rel.starts_with(".forge\\") {
                continue;
            }

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
            for f in &staged_new {
                println!("  \x1b[32mnew file:   {}\x1b[0m", f);
            }
            for f in &staged_modified {
                println!("  \x1b[32mmodified:   {}\x1b[0m", f);
            }
            for f in &staged_deleted {
                println!("  \x1b[32mdeleted:    {}\x1b[0m", f);
            }
            println!();
        }

        if !modified.is_empty() || !deleted.is_empty() {
            println!("Changes not staged for commit:");
            for f in &modified {
                println!("  \x1b[31mmodified:   {}\x1b[0m", f);
            }
            for f in &deleted {
                println!("  \x1b[31mdeleted:    {}\x1b[0m", f);
            }
            println!();
        }

        if !untracked.is_empty() {
            println!("Untracked files:");
            for f in &untracked {
                println!("  \x1b[90m{}\x1b[0m", f);
            }
            println!();
        }

        if !has_staged && modified.is_empty() && deleted.is_empty() && untracked.is_empty() && locks.is_empty() {
            println!("Nothing to report — working tree clean.");
        }
    }

    Ok(())
}
