use anyhow::{Context, Result};
use forge_core::chunk::{self, ChunkResult};
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::workspace::Workspace;
use std::path::Path;
use std::time::SystemTime;

pub fn run(paths: Vec<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let mut index = Index::load(&ws.forge_dir().join("index"))?;

    for path_str in &paths {
        let abs_path = cwd.join(path_str);
        if abs_path.is_dir() {
            add_directory(&ws, &mut index, &abs_path)?;
        } else {
            add_file(&ws, &mut index, &abs_path)?;
        }
    }

    index.save(&ws.forge_dir().join("index"))?;
    Ok(())
}

fn add_directory(ws: &Workspace, index: &mut Index, dir: &Path) -> Result<()> {
    let ignore = forge_ignore::ForgeIgnore::from_file(&ws.root.join(".forgeignore"))
        .unwrap_or_else(|_| forge_ignore::ForgeIgnore::from_str("").unwrap());

    for entry in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(&ws.root).unwrap_or(entry.path());
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if !ignore.is_ignored(&rel_str) {
                add_file(ws, index, entry.path())?;
            }
        }
    }
    Ok(())
}

fn add_file(ws: &Workspace, index: &mut Index, abs_path: &Path) -> Result<()> {
    let rel_path = abs_path
        .strip_prefix(&ws.root)
        .unwrap_or(abs_path)
        .to_string_lossy()
        .replace('\\', "/");

    let data = std::fs::read(abs_path)
        .with_context(|| format!("Failed to read {}", abs_path.display()))?;

    let metadata = std::fs::metadata(abs_path)?;
    let mtime = metadata
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    let (content_hash, object_hash, is_chunked) = match chunk::chunk_file(&data) {
        ChunkResult::WholeFile { hash, data } => {
            ws.object_store.put_blob_data(&data)?;
            (hash, hash, false)
        }
        ChunkResult::Chunked { manifest, chunks } => {
            let content_hash = ForgeHash::from_bytes(&data);
            for (hash, chunk_data) in &chunks {
                ws.object_store.put_chunk(hash, chunk_data)?;
            }
            let manifest_hash = ws.object_store.put_chunked_blob(&manifest)?;
            (content_hash, manifest_hash, true)
        }
    };

    index.set(
        rel_path.clone(),
        IndexEntry {
            hash: content_hash,
            size: data.len() as u64,
            mtime_secs: mtime.as_secs() as i64,
            mtime_nanos: mtime.subsec_nanos(),
            staged: true,
            is_chunked,
            object_hash,
        },
    );

    println!("  added: {}", rel_path);
    Ok(())
}
