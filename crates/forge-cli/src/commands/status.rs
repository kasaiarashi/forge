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

/// Server-fetched status info: locks and the remote tip for the current branch.
struct ServerInfo {
    locks: Vec<(String, String)>,
    /// Remote tip hash for the current branch, if available from the server.
    remote_tip: Option<ForgeHash>,
}

/// Fetch locks and the remote branch tip from the server in one connection.
/// Best-effort with a 2s timeout — returns defaults on failure.
fn fetch_server_info(ws: &Workspace, branch: Option<&str>) -> ServerInfo {
    let config = match ws.config() {
        Ok(c) => c,
        Err(_) => return ServerInfo { locks: vec![], remote_tip: None },
    };
    let server_url = match config.default_remote_url() {
        Some(u) => u.to_string(),
        None => return ServerInfo { locks: vec![], remote_tip: None },
    };
    let repo = if config.repo.is_empty() {
        "default".to_string()
    } else {
        config.repo.clone()
    };

    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(_) => return ServerInfo { locks: vec![], remote_tip: None },
    };

    let branch_owned = branch.map(|b| b.to_string());

    rt.block_on(async {
        use forge_proto::forge::*;

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut client = crate::client::connect_forge(&server_url).await?;

            // Fetch locks.
            let locks_resp = client
                .list_locks(ListLocksRequest {
                    repo: repo.clone(),
                    path_prefix: String::new(),
                    owner: String::new(),
                })
                .await?
                .into_inner();
            let locks: Vec<(String, String)> = locks_resp
                .locks
                .into_iter()
                .map(|l| (l.path, l.owner))
                .collect();

            // Fetch remote refs to get the current branch tip.
            let remote_tip = if let Some(ref branch) = branch_owned {
                let refs_resp = client
                    .get_refs(GetRefsRequest { repo: repo.clone() })
                    .await?
                    .into_inner();
                let ref_name = format!("refs/heads/{}", branch);
                refs_resp
                    .refs
                    .get(&ref_name)
                    .and_then(|bytes| {
                        ForgeHash::from_hex(&hex::encode(bytes)).ok()
                    })
            } else {
                None
            };

            Ok::<ServerInfo, Box<dyn std::error::Error>>(ServerInfo { locks, remote_tip })
        })
        .await;

        match result {
            Ok(Ok(info)) => info,
            _ => ServerInfo { locks: vec![], remote_tip: None },
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

    let has_staged = !staged_new.is_empty() || !staged_modified.is_empty() || !staged_deleted.is_empty();

    // Compute ahead/behind relative to the remote-tracking branch.
    let branch_name = ws.current_branch()?;
    let config = ws.config()?;
    let remote_name = config
        .default_remote()
        .map(|r| r.name.clone())
        .unwrap_or_else(|| "origin".into());

    // Fetch locks + remote branch tip from the server in one call.
    // If the server is offline, falls back to local remote-tracking refs.
    let server_info = fetch_server_info(&ws, branch_name.as_deref());
    let locks = server_info.locks;

    let (ahead, behind, remote_label) = if let Some(ref branch) = branch_name {
        // Prefer the live server tip; fall back to local remote-tracking ref.
        let remote_tip = server_info.remote_tip.or_else(|| {
            ws.get_remote_ref(&remote_name, branch).ok()
        });
        match remote_tip {
            Some(tip) if !tip.is_zero() => {
                let local_tip = ws.get_branch_tip(branch).unwrap_or(ForgeHash::ZERO);
                let (a, b) = count_ahead_behind(&ws, &local_tip, &tip);
                (a, b, Some(format!("{}/{}", remote_name, branch)))
            }
            _ => (0, 0, None),
        }
    } else {
        (0, 0, None)
    };

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
            "ahead": ahead,
            "behind": behind,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if let Some(ref branch) = branch_name {
            println!("On branch {}", branch);
        }
        if let Some(ref label) = remote_label {
            if ahead == 0 && behind == 0 {
                println!("Your branch is up to date with '{}'.", label);
            } else if ahead > 0 && behind == 0 {
                println!(
                    "Your branch is ahead of '{}' by {} commit{}.",
                    label, ahead, if ahead == 1 { "" } else { "s" }
                );
            } else if behind > 0 && ahead == 0 {
                println!(
                    "Your branch is behind '{}' by {} commit{}, and can be fast-forwarded.",
                    label, behind, if behind == 1 { "" } else { "s" }
                );
            } else {
                println!(
                    "Your branch and '{}' have diverged,\n\
                     and have {} and {} different commit{} each, respectively.",
                    label, ahead, behind, if ahead + behind == 2 { "" } else { "s" }
                );
            }
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

/// Count how many commits `local` is ahead of and behind `remote`.
///
/// Walks both chains back to find the common ancestor (merge base), then
/// counts commits on each side. Caps the walk at 1000 to avoid runaway
/// traversals on very long histories.
fn count_ahead_behind(ws: &Workspace, local: &ForgeHash, remote: &ForgeHash) -> (usize, usize) {
    if local == remote {
        return (0, 0);
    }

    // Collect ancestors of each side.
    let local_ancestors = collect_ancestors(ws, local, 1000);
    let remote_ancestors = collect_ancestors(ws, remote, 1000);

    let ahead = local_ancestors
        .iter()
        .take_while(|h| !remote_ancestors.contains(h))
        .count();
    let behind = remote_ancestors
        .iter()
        .take_while(|h| !local_ancestors.contains(h))
        .count();

    (ahead, behind)
}

/// Walk the first-parent chain from `start`, returning an ordered list of
/// commit hashes (newest first). Stops after `limit` commits.
fn collect_ancestors(ws: &Workspace, start: &ForgeHash, limit: usize) -> Vec<ForgeHash> {
    let mut result = Vec::new();
    let mut current = *start;
    for _ in 0..limit {
        if current.is_zero() {
            break;
        }
        result.push(current);
        match ws.object_store.get_snapshot(&current) {
            Ok(snap) => {
                current = snap.parents.first().copied().unwrap_or(ForgeHash::ZERO);
            }
            Err(_) => break,
        }
    }
    result
}
