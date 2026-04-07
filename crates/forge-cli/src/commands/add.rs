// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{Context, Result};
use forge_core::chunk::{self, ChunkResult};
use forge_core::compress;
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::workspace::Workspace;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

/// Pre-compressed chunk ready for direct filesystem write.
struct CompressedChunk {
    hash: ForgeHash,
    compressed: Vec<u8>,
}

pub fn run(paths: Vec<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let ignore = forge_ignore::ForgeIgnore::from_file(&ws.root.join(".forgeignore"))
        .unwrap_or_default();

    // 1. Collect all file paths first (fast walk).
    let mut file_paths: Vec<PathBuf> = Vec::new();
    for path_str in &paths {
        let abs_path = if Path::new(path_str).is_absolute() {
            PathBuf::from(path_str)
        } else {
            cwd.join(path_str)
        };

        if abs_path.is_dir() {
            let forge_dir_name = std::ffi::OsStr::new(".forge");
            for entry in walkdir::WalkDir::new(&abs_path)
                .into_iter()
                .filter_entry(|e| {
                    // Skip .forge and ignored directories entirely (don't descend).
                    if e.file_name() == forge_dir_name {
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
                            return false;
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
                        .unwrap_or(entry.path());
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    if !ignore.is_ignored(&rel_str) {
                        file_paths.push(entry.into_path());
                    }
                }
            }
        } else if abs_path.exists() {
            file_paths.push(abs_path);
        }
        // Non-existent files are handled by the deletion detection below.
    }

    // Load the index to detect unchanged files and deletions.
    let index = Index::load(&ws.forge_dir().join("index"))?;

    // Collect the set of disk files (relative paths) for deletion detection.
    // Find deleted files: in index but no longer on disk (within requested paths).
    // For "forge add .", we check all index entries. For specific paths, only those under the path.
    let is_add_all = paths.iter().any(|p| p == ".");
    let mut deleted_paths: Vec<String> = Vec::new();
    for (idx_path, _entry) in &index.entries {
        let dominated = if is_add_all {
            true
        } else {
            paths.iter().any(|p| {
                let norm = p.replace('\\', "/");
                idx_path.starts_with(&norm) || idx_path == &norm
            })
        };
        if dominated {
            let abs = ws.root.join(idx_path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if !abs.exists() {
                deleted_paths.push(idx_path.clone());
            }
        }
    }

    // Filter out files that are already tracked and unchanged.
    file_paths.retain(|abs_path| {
        let rel_path = abs_path
            .strip_prefix(&ws.root)
            .unwrap_or(abs_path)
            .to_string_lossy()
            .replace('\\', "/");

        if let Some(entry) = index.get(&rel_path) {
            // Check if content changed using mtime + size fast path.
            if let Ok(metadata) = std::fs::metadata(abs_path) {
                if let Ok(mtime) = metadata.modified().and_then(|m| {
                    Ok(m.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default())
                }) {
                    if mtime.as_secs() as i64 == entry.mtime_secs
                        && mtime.subsec_nanos() == entry.mtime_nanos
                        && metadata.len() == entry.size
                    {
                        return false; // Unchanged — skip
                    }
                    // mtime/size changed — re-hash to confirm
                    if let Ok(data) = std::fs::read(abs_path) {
                        let hash = ForgeHash::from_bytes(&data);
                        if hash == entry.hash {
                            return false; // Content identical — skip
                        }
                    }
                }
            }
        }
        true // New or modified — include
    });

    let total = file_paths.len();
    if total == 0 && deleted_paths.is_empty() {
        return Ok(());
    }

    let counter = AtomicUsize::new(0);
    let objects_dir = ws.forge_dir().join("objects");

    // 2. Parallel phase: read + hash + chunk + compress all in parallel.
    let results: Vec<Result<(String, IndexEntry, Vec<CompressedChunk>)>> = file_paths
        .par_iter()
        .map(|abs_path| {
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

            let (content_hash, object_hash, is_chunked, raw_chunks) =
                match chunk::chunk_file(&data) {
                    ChunkResult::WholeFile { hash, data } => {
                        (hash, hash, false, vec![(hash, data)])
                    }
                    ChunkResult::Chunked { manifest, chunks } => {
                        let content_hash = ForgeHash::from_bytes(&data);
                        let manifest_data = bincode::serialize(&manifest)
                            .map_err(|e| anyhow::anyhow!("serialize: {}", e))?;
                        let mut tag = vec![2u8];
                        tag.extend_from_slice(&manifest_data);
                        let manifest_hash = ForgeHash::from_bytes(&tag);
                        let mut all = chunks;
                        all.push((manifest_hash, tag));
                        (content_hash, manifest_hash, true, all)
                    }
                };

            // Compress all chunks in this thread (most expensive CPU work).
            let compressed_chunks: Vec<CompressedChunk> = raw_chunks
                .into_iter()
                .map(|(hash, raw)| {
                    let compressed = compress::compress(&raw)?;
                    Ok(CompressedChunk { hash, compressed })
                })
                .collect::<Result<Vec<_>>>()?;

            let entry = IndexEntry {
                hash: content_hash,
                size: data.len() as u64,
                mtime_secs: mtime.as_secs() as i64,
                mtime_nanos: mtime.subsec_nanos(),
                staged: true,
                is_chunked,
                object_hash,
            };

            let done = counter.fetch_add(1, Ordering::Relaxed) + 1;
            if total > 50 && done % 500 == 0 {
                eprintln!("  [{}/{}] files hashed & compressed...", done, total);
            }

            Ok((rel_path, entry, compressed_chunks))
        })
        .collect();

    // 3. Sequential phase: write pre-compressed data to disk + update index.
    let mut index = Index::load(&ws.forge_dir().join("index"))?;

    for result in results {
        let (rel_path, entry, compressed_chunks) = result?;

        for cc in &compressed_chunks {
            let hex = cc.hash.to_hex();
            let path = objects_dir.join(&hex[..2]).join(&hex[2..]);
            if path.exists() {
                continue; // dedup
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, &cc.compressed)?;
            std::fs::rename(&tmp, &path)?;
        }

        index.set(rel_path, entry);
    }

    // Stage deleted files: mark with ZERO hash to indicate intentional deletion.
    for del_path in &deleted_paths {
        if let Some(entry) = index.entries.get_mut(del_path) {
            entry.staged = true;
            entry.hash = ForgeHash::ZERO;
            entry.object_hash = ForgeHash::ZERO;
            entry.size = 0;
        }
    }

    index.save(&ws.forge_dir().join("index"))?;

    Ok(())
}
