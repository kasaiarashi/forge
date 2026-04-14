//! Source-selection: working dir vs index (unstaged), index vs HEAD (staged),
//! and HEAD vs arbitrary commit.
//!
//! Returns a list of [`FileDiff`] records for the formatters to consume.

use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::time::SystemTime;

use forge_core::diff::{diff_maps, flatten_tree, DiffEntry};
use forge_core::hash::ForgeHash;
use forge_core::index::Index;
use forge_core::workspace::Workspace;
use forge_diff::asset_paths::is_binary;
use forge_diff::format::FileDiff;

use super::blob::{matches_filter, read_blob_content};

/// Max file size for text diff (10 MiB). Files larger than this are treated
/// as binary without loading their full content.
const MAX_DIFF_SIZE: u64 = 10 * 1024 * 1024;

/// Working directory vs index (unstaged changes).
pub fn diff_unstaged(ws: &Workspace, index: &Index, filter: &[String]) -> Result<Vec<FileDiff>> {
    let mut diffs = Vec::new();

    for (path, entry) in &index.entries {
        if entry.staged {
            continue;
        }
        if !matches_filter(path, filter) {
            continue;
        }

        let abs_path = ws
            .root
            .join(path.replace('/', std::path::MAIN_SEPARATOR_STR));

        if !abs_path.exists() {
            let old_data = read_blob_content(ws, &entry.object_hash)?;
            diffs.push(FileDiff {
                path: path.clone(),
                status: "deleted",
                binary: is_binary(&old_data),
                old_content: old_data,
                new_content: vec![],
            });
            continue;
        }

        // Fast path: mtime + size check.
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

        // Skip large files early — treat as binary without loading full content.
        if metadata.len() > MAX_DIFF_SIZE || entry.size > MAX_DIFF_SIZE {
            let new_data = std::fs::read(&abs_path)?;
            let hash = ForgeHash::from_bytes(&new_data);
            if hash != entry.hash {
                diffs.push(FileDiff {
                    path: path.clone(),
                    status: "modified",
                    binary: true,
                    old_content: vec![],
                    new_content: vec![],
                });
            }
            continue;
        }

        let new_data = std::fs::read(&abs_path)?;
        let hash = ForgeHash::from_bytes(&new_data);
        if hash == entry.hash {
            continue;
        }

        let old_data = read_blob_content(ws, &entry.object_hash)?;
        diffs.push(FileDiff {
            path: path.clone(),
            status: "modified",
            binary: is_binary(&old_data) || is_binary(&new_data),
            old_content: old_data,
            new_content: new_data,
        });
    }

    Ok(diffs)
}

/// Index vs HEAD (staged changes).
pub fn diff_staged(ws: &Workspace, index: &Index, filter: &[String]) -> Result<Vec<FileDiff>> {
    let head_hash = ws.head_snapshot()?;
    let head_map = build_file_map(ws, &head_hash)?;

    let mut diffs = Vec::new();

    for (path, entry) in &index.entries {
        if !entry.staged {
            continue;
        }
        if !matches_filter(path, filter) {
            continue;
        }

        if entry.hash == ForgeHash::ZERO {
            // Staged deletion.
            if let Some((old_hash, _)) = head_map.get(path) {
                let old_data = read_blob_content(ws, old_hash)?;
                diffs.push(FileDiff {
                    path: path.clone(),
                    status: "deleted",
                    binary: is_binary(&old_data),
                    old_content: old_data,
                    new_content: vec![],
                });
            }
            continue;
        }

        let new_data = read_blob_content(ws, &entry.object_hash)?;
        let old_data = match head_map.get(path) {
            Some((old_hash, _)) => read_blob_content(ws, old_hash)?,
            None => vec![],
        };

        let status = if head_map.contains_key(path) {
            "modified"
        } else {
            "added"
        };

        if old_data == new_data {
            continue;
        }

        diffs.push(FileDiff {
            path: path.clone(),
            status,
            binary: is_binary(&old_data) || is_binary(&new_data),
            old_content: old_data,
            new_content: new_data,
        });
    }

    Ok(diffs)
}

/// Diff between HEAD and a specific commit.
pub fn diff_commit(ws: &Workspace, commit_str: &str, filter: &[String]) -> Result<Vec<FileDiff>> {
    let target_hash = ws.resolve_ref(commit_str)?;
    let head_hash = ws.head_snapshot()?;

    if head_hash.is_zero() {
        bail!("No commits yet.");
    }

    let head_map = build_file_map(ws, &head_hash)?;
    let target_map = build_file_map(ws, &target_hash)?;

    let changes = diff_maps(&head_map, &target_map);
    let mut diffs = Vec::new();

    for change in changes {
        let (path, status, old_hash, new_hash) = match &change {
            DiffEntry::Added { path, hash, .. } => {
                (path.clone(), "added", None, Some(*hash))
            }
            DiffEntry::Deleted { path, hash, .. } => {
                (path.clone(), "deleted", Some(*hash), None)
            }
            DiffEntry::Modified {
                path,
                old_hash,
                new_hash,
                ..
            } => (path.clone(), "modified", Some(*old_hash), Some(*new_hash)),
        };

        if !matches_filter(&path, filter) {
            continue;
        }

        let old_data = match old_hash {
            Some(h) => read_blob_content(ws, &h)?,
            None => vec![],
        };
        let new_data = match new_hash {
            Some(h) => read_blob_content(ws, &h)?,
            None => vec![],
        };

        diffs.push(FileDiff {
            path,
            status,
            binary: is_binary(&old_data) || is_binary(&new_data),
            old_content: old_data,
            new_content: new_data,
        });
    }

    Ok(diffs)
}

fn build_file_map(
    ws: &Workspace,
    hash: &ForgeHash,
) -> Result<BTreeMap<String, (ForgeHash, u64)>> {
    if hash.is_zero() {
        return Ok(BTreeMap::new());
    }
    let snap = ws.object_store.get_snapshot(hash)?;
    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();
    match ws.object_store.get_tree(&snap.tree) {
        Ok(tree) => Ok(flatten_tree(&tree, "", &get_tree)),
        Err(_) => Ok(BTreeMap::new()),
    }
}
