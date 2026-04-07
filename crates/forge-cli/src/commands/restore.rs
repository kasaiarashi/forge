use anyhow::{bail, Result};
use forge_core::diff::flatten_tree;
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::workspace::Workspace;
use std::time::SystemTime;

fn matches_filter_paths(path: &str, filter: &[String], match_all: bool) -> bool {
    if match_all {
        return true;
    }
    filter.iter().any(|f| path == f || path.starts_with(&format!("{}/", f)))
}

/// Restore working tree files from the index or a commit.
///
/// - `forge restore <paths>` — restore from index (discard unstaged changes)
/// - `forge restore --staged <paths>` — unstage (delegates to unstage command)
/// - `forge restore --source <commit> <paths>` — restore from a specific commit
pub fn run(staged: bool, source: Option<String>, paths: Vec<String>) -> Result<()> {
    if staged {
        return crate::commands::unstage::run(paths);
    }

    if paths.is_empty() {
        bail!("Nothing specified, nothing restored. Use: forge restore <path>");
    }

    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let mut index = Index::load(&ws.forge_dir().join("index"))?;

    // Normalize requested paths. "." means all files.
    let match_all = paths.iter().any(|p| p == ".");
    let normalized: Vec<String> = paths
        .iter()
        .map(|p| p.replace('\\', "/").trim_start_matches("./").to_string())
        .collect();

    let mut restored = 0usize;

    if let Some(ref src) = source {
        // Restore from a specific commit.
        let commit_hash = ws
            .get_branch_tip(src)
            .or_else(|_| ForgeHash::from_hex(src))?;

        if commit_hash.is_zero() {
            bail!("No commits to restore from.");
        }

        let snap = ws.object_store.get_snapshot(&commit_hash)?;
        let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();
        let tree = ws.object_store.get_tree(&snap.tree)?;
        let file_map = flatten_tree(&tree, "", &get_tree);

        for req in &normalized {
            let matching: Vec<_> = file_map
                .iter()
                .filter(|(p, _)| match_all || *p == req || p.starts_with(&format!("{}/", req)))
                .map(|(p, (h, s))| (p.clone(), *h, *s))
                .collect();

            if matching.is_empty() {
                eprintln!("warning: path '{}' not found in {}", req, commit_hash.short());
                continue;
            }

            for (file_path, object_hash, size) in matching {
                let content = read_blob_content(&ws, &object_hash)?;
                write_and_update_index(&ws, &mut index, &file_path, &content, object_hash, size)?;
                restored += 1;
                println!("Restored '{}'", file_path);
            }
        }
    } else {
        // Restore from the index (discard working directory changes).
        for req in &normalized {
            let matching: Vec<_> = index
                .entries
                .iter()
                .filter(|(p, _)| match_all || *p == req || p.starts_with(&format!("{}/", req)))
                .map(|(p, e)| (p.clone(), e.clone()))
                .collect();

            if matching.is_empty() {
                eprintln!("warning: path '{}' not in the index", req);
                continue;
            }

            for (file_path, entry) in matching {
                if entry.hash == ForgeHash::ZERO {
                    // Staged deletion — to restore working tree, need content from HEAD.
                    let content = restore_from_head(&ws, &file_path)?;
                    let object_hash = entry.object_hash;
                    let size = content.len() as u64;
                    write_and_update_index(&ws, &mut index, &file_path, &content, object_hash, size)?;
                    restored += 1;
                    println!("Restored '{}'", file_path);
                    continue;
                }

                // Skip files that haven't actually changed (mtime+size fast path).
                let abs_path = ws.root.join(file_path.replace('/', std::path::MAIN_SEPARATOR_STR));
                if abs_path.exists() {
                    let metadata = std::fs::metadata(&abs_path)?;
                    let mtime = metadata
                        .modified()?
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default();

                    if mtime.as_secs() as i64 == entry.mtime_secs
                        && mtime.subsec_nanos() == entry.mtime_nanos
                        && metadata.len() == entry.size
                    {
                        continue;
                    }

                    // Re-hash to confirm it actually changed.
                    let disk_data = std::fs::read(&abs_path)?;
                    if ForgeHash::from_bytes(&disk_data) == entry.hash {
                        continue;
                    }
                }

                let content = read_blob_content(&ws, &entry.object_hash)?;
                write_and_update_index(
                    &ws,
                    &mut index,
                    &file_path,
                    &content,
                    entry.object_hash,
                    entry.size,
                )?;
                restored += 1;
                println!("Restored '{}'", file_path);
            }
        }
    }

    // Remove untracked files.
    let ignore = forge_ignore::ForgeIgnore::from_file(&ws.root.join(".forgeignore"))
        .unwrap_or_else(|_| forge_ignore::ForgeIgnore::from_str("").unwrap());
    let mut removed = 0usize;

    for entry in walkdir::WalkDir::new(&ws.root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(&ws.root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");

        if rel.starts_with(".forge/") {
            continue;
        }
        if ignore.is_ignored(&rel) {
            continue;
        }
        if index.entries.contains_key(&rel) {
            continue;
        }
        if !matches_filter_paths(&rel, &normalized, match_all) {
            continue;
        }

        std::fs::remove_file(entry.path())?;
        removed += 1;
        println!("Removed '{}'", rel);
    }

    index.save(&ws.forge_dir().join("index"))?;

    if restored == 0 && removed == 0 {
        println!("Nothing to restore — working tree clean.");
    } else {
        if restored > 0 {
            println!("Restored {} file(s)", restored);
        }
        if removed > 0 {
            println!("Removed {} untracked file(s)", removed);
        }
    }
    Ok(())
}

fn write_and_update_index(
    ws: &Workspace,
    index: &mut Index,
    file_path: &str,
    content: &[u8],
    object_hash: ForgeHash,
    size: u64,
) -> Result<()> {
    let abs_path = ws
        .root
        .join(file_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    if let Some(parent) = abs_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&abs_path, content)?;

    let mtime = std::fs::metadata(&abs_path)?
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    index.set(
        file_path.to_string(),
        IndexEntry {
            hash: ForgeHash::from_bytes(content),
            size,
            mtime_secs: mtime.as_secs() as i64,
            mtime_nanos: mtime.subsec_nanos(),
            staged: false,
            is_chunked: false,
            object_hash,
        },
    );

    Ok(())
}

/// Restore a file's content from the HEAD commit.
fn restore_from_head(ws: &Workspace, file_path: &str) -> Result<Vec<u8>> {
    let head = ws.head_snapshot()?;
    if head.is_zero() {
        bail!("No commits yet — cannot restore '{}'", file_path);
    }

    let snap = ws.object_store.get_snapshot(&head)?;
    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();
    let tree = ws.object_store.get_tree(&snap.tree)?;
    let file_map = flatten_tree(&tree, "", &get_tree);

    match file_map.get(file_path) {
        Some((hash, _)) => read_blob_content(ws, hash),
        None => bail!("Path '{}' not found in HEAD", file_path),
    }
}

/// Read blob content, handling both small and chunked blobs.
fn read_blob_content(ws: &Workspace, object_hash: &ForgeHash) -> Result<Vec<u8>> {
    let data = ws
        .object_store
        .chunks
        .get(object_hash)
        .map_err(|e| anyhow::anyhow!("Failed to read object {}: {}", object_hash.short(), e))?;

    if data.is_empty() {
        return Ok(data);
    }

    if data[0] == 2 {
        let manifest: forge_core::object::blob::ChunkedBlob = bincode::deserialize(&data[1..])
            .map_err(|e| anyhow::anyhow!("Failed to deserialize manifest: {}", e))?;
        let content = forge_core::chunk::reassemble_chunks(&manifest, |h| {
            ws.object_store.chunks.get(h).ok()
        })
        .ok_or_else(|| anyhow::anyhow!("Failed to reassemble chunked blob"))?;
        Ok(content)
    } else {
        Ok(data)
    }
}
