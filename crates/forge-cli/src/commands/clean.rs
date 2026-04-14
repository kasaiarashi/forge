use anyhow::Result;
use forge_core::index::Index;
use forge_core::workspace::Workspace;

pub fn run(force: bool, directories: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let index = Index::load(&ws.forge_dir().join("index"))?;
    let ignore = forge_ignore::ForgeIgnore::from_file(&ws.root.join(".forgeignore"))
        .unwrap_or_default();

    let mut untracked_files: Vec<String> = Vec::new();
    let mut untracked_dirs: Vec<String> = Vec::new();

    // Collect untracked files.
    for entry in walkdir::WalkDir::new(&ws.root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let rel = entry
            .path()
            .strip_prefix(&ws.root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");

        if rel.starts_with(".forge/") || rel.starts_with(".forge\\") || rel.is_empty() {
            continue;
        }

        if ignore.is_ignored(&rel) {
            continue;
        }

        if entry.file_type().is_file() {
            if !index.entries.contains_key(&rel) {
                untracked_files.push(rel);
            }
        }
    }

    // Find untracked directories (directories containing no tracked files).
    if directories {
        let mut tracked_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
        for key in index.entries.keys() {
            let mut current = String::new();
            for component in key.split('/') {
                if !current.is_empty() {
                    current.push('/');
                }
                current.push_str(component);
                tracked_dirs.insert(current.clone());
            }
        }

        for entry in walkdir::WalkDir::new(&ws.root)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_dir() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&ws.root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");

            if rel.starts_with(".forge") || rel.is_empty() {
                continue;
            }

            if ignore.is_ignored(&rel) {
                continue;
            }

            // Check if any tracked file lives under this directory.
            let has_tracked = index.entries.keys().any(|k| k.starts_with(&format!("{}/", rel)));
            if !has_tracked {
                untracked_dirs.push(rel);
            }
        }

        // Sort by depth descending so we remove children before parents.
        untracked_dirs.sort_by(|a, b| {
            let depth_a = a.matches('/').count();
            let depth_b = b.matches('/').count();
            depth_b.cmp(&depth_a).then(a.cmp(b))
        });

        // Filter out dirs that are subdirectories of other untracked dirs.
        let mut top_level_dirs: Vec<String> = Vec::new();
        for dir in &untracked_dirs {
            let is_sub = top_level_dirs.iter().any(|parent| dir.starts_with(&format!("{}/", parent)));
            if !is_sub {
                top_level_dirs.push(dir.clone());
            }
        }
        // For deletion we still use the depth-sorted full list, but for display use top-level.
        // Actually, just keep full list for deletion (depth-sorted) and top-level for display.
        if !force {
            untracked_dirs = top_level_dirs;
        }
    }

    if untracked_files.is_empty() && untracked_dirs.is_empty() {
        println!("Nothing to clean — working tree is clean.");
        return Ok(());
    }

    if !force {
        println!("Would remove the following untracked files:");
        for f in &untracked_files {
            println!("  {}", f);
        }
        if directories {
            for d in &untracked_dirs {
                println!("  {}/", d);
            }
        }
        println!();
        println!("Use -f to actually delete them.");
        return Ok(());
    }

    // Actually delete.
    for f in &untracked_files {
        let abs = ws.root.join(f.replace('/', std::path::MAIN_SEPARATOR_STR));
        if abs.exists() {
            std::fs::remove_file(&abs)?;
            println!("Removing {}", f);
        }
    }

    if directories {
        // Re-collect and remove untracked directories depth-first.
        let mut dirs_to_remove: Vec<String> = Vec::new();
        for entry in walkdir::WalkDir::new(&ws.root)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_dir() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&ws.root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");

            if rel.starts_with(".forge") || rel.is_empty() {
                continue;
            }

            if ignore.is_ignored(&rel) {
                continue;
            }

            let has_tracked = index.entries.keys().any(|k| k.starts_with(&format!("{}/", rel)));
            if !has_tracked {
                dirs_to_remove.push(rel);
            }
        }

        // Sort deepest first.
        dirs_to_remove.sort_by(|a, b| {
            let depth_a = a.matches('/').count();
            let depth_b = b.matches('/').count();
            depth_b.cmp(&depth_a).then(a.cmp(b))
        });

        for d in &dirs_to_remove {
            let abs = ws.root.join(d.replace('/', std::path::MAIN_SEPARATOR_STR));
            if abs.exists() && abs.is_dir() {
                // Only remove if empty (files were already removed above).
                if std::fs::remove_dir(&abs).is_ok() {
                    println!("Removing {}/", d);
                }
            }
        }
    }

    Ok(())
}
