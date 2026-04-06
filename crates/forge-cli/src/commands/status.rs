use anyhow::Result;
use forge_core::hash::ForgeHash;
use forge_core::index::Index;
use forge_core::workspace::Workspace;
use std::time::SystemTime;

pub fn run(json: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let index = Index::load(&ws.forge_dir().join("index"))?;
    let ignore = forge_ignore::ForgeIgnore::from_file(&ws.root.join(".forgeignore"))
        .unwrap_or_else(|_| forge_ignore::ForgeIgnore::from_str("").unwrap());

    let mut staged = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    let mut untracked = Vec::new();

    // Check all index entries against working tree.
    let mut seen = std::collections::HashSet::new();
    for (path, entry) in &index.entries {
        seen.insert(path.clone());
        let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));

        if !abs_path.exists() {
            deleted.push(path.clone());
            continue;
        }

        if entry.staged {
            staged.push(path.clone());
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

    if json {
        let output = serde_json::json!({
            "staged": staged,
            "modified": modified,
            "deleted": deleted,
            "untracked": untracked,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if let Some(branch) = ws.current_branch()? {
            println!("On branch {}", branch);
        }
        println!();

        if !staged.is_empty() {
            println!("Changes to be committed:");
            for f in &staged {
                println!("  \x1b[32m  staged: {}\x1b[0m", f);
            }
            println!();
        }

        if !modified.is_empty() {
            println!("Modified files:");
            for f in &modified {
                println!("  \x1b[33mmodified: {}\x1b[0m", f);
            }
            println!();
        }

        if !deleted.is_empty() {
            println!("Deleted files:");
            for f in &deleted {
                println!("  \x1b[31m deleted: {}\x1b[0m", f);
            }
            println!();
        }

        if !untracked.is_empty() {
            println!("Untracked files:");
            for f in &untracked {
                println!("  \x1b[90m     new: {}\x1b[0m", f);
            }
            println!();
        }

        if staged.is_empty() && modified.is_empty() && deleted.is_empty() && untracked.is_empty() {
            println!("Nothing to report — working tree clean.");
        }
    }

    Ok(())
}
