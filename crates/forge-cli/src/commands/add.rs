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
        .unwrap_or_else(|_| forge_ignore::ForgeIgnore::from_str("").unwrap());

    // 1. Collect all file paths first (fast walk).
    let mut file_paths: Vec<PathBuf> = Vec::new();
    for path_str in &paths {
        let abs_path = if Path::new(path_str).is_absolute() {
            PathBuf::from(path_str)
        } else {
            cwd.join(path_str)
        };

        if abs_path.is_dir() {
            for entry in walkdir::WalkDir::new(&abs_path)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.file_type().is_file() {
                    let rel = entry
                        .path()
                        .strip_prefix(&ws.root)
                        .unwrap_or(entry.path());
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    if !ignore.is_ignored(&rel_str) && !rel_str.starts_with(".forge/") {
                        file_paths.push(entry.into_path());
                    }
                }
            }
        } else {
            file_paths.push(abs_path);
        }
    }

    let total = file_paths.len();
    if total == 0 {
        println!("No files to add.");
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
    let mut added = 0usize;
    let mut skipped = 0usize;

    for result in results {
        let (rel_path, entry, compressed_chunks) = result?;

        for cc in &compressed_chunks {
            let hex = cc.hash.to_hex();
            let path = objects_dir.join(&hex[..2]).join(&hex[2..]);
            if path.exists() {
                skipped += 1;
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
        added += 1;
    }

    index.save(&ws.forge_dir().join("index"))?;

    if skipped > 0 {
        println!("Added {} file(s) ({} objects deduplicated)", added, skipped);
    } else {
        println!("Added {} file(s)", added);
    }

    Ok(())
}
