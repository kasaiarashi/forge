use anyhow::{bail, Result};
use forge_core::hash::ForgeHash;
use forge_core::index::Index;
use forge_core::workspace::Workspace;
use std::path::Path;

pub fn run(paths: Vec<String>, cached: bool, recursive: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let mut index = Index::load(&ws.forge_dir().join("index"))?;

    // Normalize each user-supplied path to a workspace-relative POSIX
    // path. If the path is a tracked file it's used as-is; if it's a
    // directory (only allowed when -r is set), expand to every tracked
    // entry whose key starts with `<dir>/`. We build the full target
    // list first so a partial failure doesn't leave the index in a
    // half-mutated state.
    let mut targets: Vec<String> = Vec::new();

    for path_str in &paths {
        let rel_path = if Path::new(path_str).is_absolute() {
            Path::new(path_str)
                .strip_prefix(&ws.root)
                .unwrap_or(Path::new(path_str))
                .to_string_lossy()
                .replace('\\', "/")
        } else {
            let abs = cwd.join(path_str);
            abs.strip_prefix(&ws.root)
                .unwrap_or(Path::new(path_str))
                .to_string_lossy()
                .replace('\\', "/")
        };
        let rel_path = rel_path.trim_end_matches('/').to_string();

        if index.get(&rel_path).is_some() {
            targets.push(rel_path);
            continue;
        }

        // Not a tracked file — treat as a directory when -r is set and
        // at least one tracked path lives under it. Without -r this is
        // an error, matching git's "not removing ... recursively without -r".
        let dir_prefix = format!("{}/", rel_path);
        let matches: Vec<String> = index
            .entries
            .keys()
            .filter(|k| k.starts_with(&dir_prefix))
            .cloned()
            .collect();

        if matches.is_empty() {
            bail!("pathspec '{}' did not match any tracked files", rel_path);
        }
        if !recursive {
            bail!(
                "not removing '{}' recursively without -r",
                rel_path
            );
        }
        targets.extend(matches);
    }

    for rel_path in &targets {
        if !cached {
            let abs_path = ws.root.join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if abs_path.exists() {
                std::fs::remove_file(&abs_path)?;
            }
        }

        // Stage the deletion by setting hash to ZERO.
        if let Some(entry) = index.entries.get_mut(rel_path) {
            entry.hash = ForgeHash::ZERO;
            entry.object_hash = ForgeHash::ZERO;
            entry.size = 0;
            entry.staged = true;
        }

        println!("rm '{}'", rel_path);
    }

    index.save(&ws.forge_dir().join("index"))?;

    Ok(())
}
