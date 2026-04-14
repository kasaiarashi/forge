use anyhow::{bail, Result};
use forge_core::index::Index;
use forge_core::workspace::Workspace;
use std::path::Path;

pub fn run(source: String, dest: String) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let mut index = Index::load(&ws.forge_dir().join("index"))?;

    let src_rel = if Path::new(&source).is_absolute() {
        Path::new(&source)
            .strip_prefix(&ws.root)
            .unwrap_or(Path::new(&source))
            .to_string_lossy()
            .replace('\\', "/")
    } else {
        let abs = cwd.join(&source);
        abs.strip_prefix(&ws.root)
            .unwrap_or(Path::new(&source))
            .to_string_lossy()
            .replace('\\', "/")
    };

    let dst_rel = if Path::new(&dest).is_absolute() {
        Path::new(&dest)
            .strip_prefix(&ws.root)
            .unwrap_or(Path::new(&dest))
            .to_string_lossy()
            .replace('\\', "/")
    } else {
        let abs = cwd.join(&dest);
        abs.strip_prefix(&ws.root)
            .unwrap_or(Path::new(&dest))
            .to_string_lossy()
            .replace('\\', "/")
    };

    // Verify source is tracked.
    let entry = match index.get(&src_rel) {
        Some(e) => e.clone(),
        None => bail!("pathspec '{}' did not match any tracked files", src_rel),
    };

    let src_abs = ws.root.join(src_rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    let dst_abs = ws.root.join(dst_rel.replace('/', std::path::MAIN_SEPARATOR_STR));

    if !src_abs.exists() {
        bail!("source file '{}' does not exist", source);
    }

    if dst_abs.exists() {
        bail!("destination '{}' already exists", dest);
    }

    // Create parent directories if needed.
    if let Some(parent) = dst_abs.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Move on disk.
    std::fs::rename(&src_abs, &dst_abs)?;

    // Update index: remove old, add new with staged=true.
    index.entries.remove(&src_rel);

    // Stage deletion for old path.
    use forge_core::hash::ForgeHash;
    use forge_core::index::IndexEntry;
    index.set(
        src_rel.clone(),
        IndexEntry {
            hash: ForgeHash::ZERO,
            object_hash: ForgeHash::ZERO,
            size: 0,
            mtime_secs: 0,
            mtime_nanos: 0,
            staged: true,
            is_chunked: false,
        },
    );

    // Stage new entry.
    let mut new_entry = entry;
    new_entry.staged = true;
    index.set(dst_rel.clone(), new_entry);

    index.save(&ws.forge_dir().join("index"))?;

    println!("renamed '{}' -> '{}'", src_rel, dst_rel);

    Ok(())
}
