use anyhow::{bail, Result};
use forge_core::hash::ForgeHash;
use forge_core::index::Index;
use forge_core::workspace::Workspace;
use std::path::Path;

pub fn run(paths: Vec<String>, cached: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let mut index = Index::load(&ws.forge_dir().join("index"))?;

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

        if index.get(&rel_path).is_none() {
            bail!("pathspec '{}' did not match any tracked files", rel_path);
        }

        if !cached {
            let abs_path = ws.root.join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if abs_path.exists() {
                std::fs::remove_file(&abs_path)?;
            }
        }

        // Stage the deletion by setting hash to ZERO.
        if let Some(entry) = index.entries.get_mut(&rel_path) {
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
