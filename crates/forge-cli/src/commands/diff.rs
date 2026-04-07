use anyhow::{bail, Result};
use forge_core::diff::{diff_maps, flatten_tree, DiffEntry};
use forge_core::hash::ForgeHash;
use forge_core::index::Index;
use forge_core::workspace::Workspace;
use similar::ChangeTag;
use std::collections::BTreeMap;
use std::time::SystemTime;

pub fn run(commit: Option<String>, staged: bool, stat: bool, paths: Vec<String>, json: bool) -> Result<()> {
    if staged && commit.is_some() {
        bail!("Cannot use --staged with --commit");
    }

    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let index = Index::load(&ws.forge_dir().join("index"))?;

    let filter: Vec<String> = paths
        .iter()
        .map(|p| p.replace('\\', "/").trim_start_matches("./").to_string())
        .collect();

    let file_diffs = if let Some(ref commit_str) = commit {
        diff_commit(&ws, commit_str, &filter)?
    } else if staged {
        diff_staged(&ws, &index, &filter)?
    } else {
        diff_unstaged(&ws, &index, &filter)?
    };

    if json {
        print_json(&file_diffs)?;
    } else if stat {
        print_stat(&file_diffs);
    } else {
        print_colored(&file_diffs);
    }

    Ok(())
}

struct FileDiff {
    path: String,
    status: &'static str,
    binary: bool,
    old_content: Vec<u8>,
    new_content: Vec<u8>,
}

fn matches_filter(path: &str, filter: &[String]) -> bool {
    if filter.is_empty() {
        return true;
    }
    filter.iter().any(|f| path == f || path.starts_with(&format!("{}/", f)))
}

fn is_binary(data: &[u8]) -> bool {
    data.iter().take(8192).any(|&b| b == 0)
}

/// Max file size for text diff (10 MiB). Files larger than this are treated as binary.
const MAX_DIFF_SIZE: u64 = 10 * 1024 * 1024;

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
        // ChunkedBlob manifest.
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

/// Working directory vs index (unstaged changes).
fn diff_unstaged(ws: &Workspace, index: &Index, filter: &[String]) -> Result<Vec<FileDiff>> {
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
            // Deleted file.
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
            // Still need to verify it actually changed.
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

        // Re-hash to confirm.
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
fn diff_staged(ws: &Workspace, index: &Index, filter: &[String]) -> Result<Vec<FileDiff>> {
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

        // Skip if content is identical.
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
fn diff_commit(ws: &Workspace, commit_str: &str, filter: &[String]) -> Result<Vec<FileDiff>> {
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

fn print_colored(diffs: &[FileDiff]) {
    for diff in diffs {
        if diff.binary {
            println!(
                "Binary files a/{} and b/{} differ",
                diff.path, diff.path
            );
            continue;
        }

        let old_str = String::from_utf8_lossy(&diff.old_content);
        let new_str = String::from_utf8_lossy(&diff.new_content);

        let text_diff = similar::TextDiff::from_lines(old_str.as_ref(), new_str.as_ref());

        let ops = text_diff.grouped_ops(3);
        if ops.is_empty() {
            continue;
        }

        // File header.
        println!(
            "\x1b[1mdiff --forge a/{} b/{}\x1b[0m",
            diff.path, diff.path
        );
        match diff.status {
            "added" => {
                println!("\x1b[1m--- /dev/null\x1b[0m");
                println!("\x1b[1m+++ b/{}\x1b[0m", diff.path);
            }
            "deleted" => {
                println!("\x1b[1m--- a/{}\x1b[0m", diff.path);
                println!("\x1b[1m+++ /dev/null\x1b[0m");
            }
            _ => {
                println!("\x1b[1m--- a/{}\x1b[0m", diff.path);
                println!("\x1b[1m+++ b/{}\x1b[0m", diff.path);
            }
        }

        for group in &ops {
            // Compute hunk header.
            let first_op = group.first().unwrap();
            let last_op = group.last().unwrap();
            let old_start_idx = first_op.old_range().start;
            let new_start_idx = first_op.new_range().start;
            let old_end = last_op.old_range().end;
            let new_end = last_op.new_range().end;
            let old_start = old_start_idx + 1;
            let old_count = old_end - old_start_idx;
            let new_start = new_start_idx + 1;
            let new_count = new_end - new_start_idx;

            println!(
                "\x1b[36m@@ -{},{} +{},{} @@\x1b[0m",
                old_start, old_count, new_start, new_count
            );

            for op in group {
                for change in text_diff.iter_changes(op) {
                    let (prefix, color) = match change.tag() {
                        ChangeTag::Equal => (" ", ""),
                        ChangeTag::Delete => ("-", "\x1b[31m"),
                        ChangeTag::Insert => ("+", "\x1b[32m"),
                    };
                    let reset = if color.is_empty() { "" } else { "\x1b[0m" };
                    let value = change.value();
                    if value.ends_with('\n') {
                        print!("{}{}{}{}", color, prefix, value, reset);
                    } else {
                        println!("{}{}{}{}", color, prefix, value, reset);
                    }
                }
            }
        }
    }
}

fn print_json(diffs: &[FileDiff]) -> Result<()> {
    let mut entries = Vec::new();

    for diff in diffs {
        if diff.binary {
            entries.push(serde_json::json!({
                "path": diff.path,
                "status": diff.status,
                "binary": true,
                "hunks": [],
            }));
            continue;
        }

        let old_str = String::from_utf8_lossy(&diff.old_content);
        let new_str = String::from_utf8_lossy(&diff.new_content);
        let text_diff = similar::TextDiff::from_lines(old_str.as_ref(), new_str.as_ref());

        let mut hunks = Vec::new();
        for group in text_diff.grouped_ops(3) {
            let first_op = group.first().unwrap();
            let last_op = group.last().unwrap();
            let old_start = first_op.old_range().start;
            let new_start = first_op.new_range().start;
            let old_end = last_op.old_range().end;
            let new_end = last_op.new_range().end;

            let header = format!(
                "@@ -{},{} +{},{} @@",
                old_start + 1,
                old_end - old_start,
                new_start + 1,
                new_end - new_start,
            );

            let mut lines = Vec::new();
            for op in &group {
                for change in text_diff.iter_changes(op) {
                    let tag = match change.tag() {
                        ChangeTag::Equal => "context",
                        ChangeTag::Delete => "delete",
                        ChangeTag::Insert => "add",
                    };
                    lines.push(serde_json::json!({
                        "tag": tag,
                        "content": change.value(),
                    }));
                }
            }

            hunks.push(serde_json::json!({
                "header": header,
                "lines": lines,
            }));
        }

        entries.push(serde_json::json!({
            "path": diff.path,
            "status": diff.status,
            "binary": false,
            "hunks": hunks,
        }));
    }

    println!("{}", serde_json::to_string_pretty(&entries)?);
    Ok(())
}

fn print_stat(diffs: &[FileDiff]) {
    let mut total_insertions = 0usize;
    let mut total_deletions = 0usize;
    let mut max_path_len = 0usize;

    // First pass: compute stats.
    let mut stats: Vec<(&str, &str, usize, usize)> = Vec::new();
    for diff in diffs {
        if diff.binary {
            stats.push((&diff.path, "Bin", 0, 0));
            if diff.path.len() > max_path_len {
                max_path_len = diff.path.len();
            }
            continue;
        }

        let old_str = String::from_utf8_lossy(&diff.old_content);
        let new_str = String::from_utf8_lossy(&diff.new_content);
        let text_diff = similar::TextDiff::from_lines(old_str.as_ref(), new_str.as_ref());

        let mut insertions = 0usize;
        let mut deletions = 0usize;
        for op in text_diff.ops() {
            for change in text_diff.iter_changes(op) {
                match change.tag() {
                    ChangeTag::Insert => insertions += 1,
                    ChangeTag::Delete => deletions += 1,
                    ChangeTag::Equal => {}
                }
            }
        }
        total_insertions += insertions;
        total_deletions += deletions;
        stats.push((&diff.path, diff.status, insertions, deletions));
        if diff.path.len() > max_path_len {
            max_path_len = diff.path.len();
        }
    }

    // Second pass: print.
    for (path, status, ins, del) in &stats {
        if *status == "Bin" {
            println!(" {:<width$} | Bin", path, width = max_path_len);
        } else {
            let total = ins + del;
            println!(
                " {:<width$} | {:>4} {}",
                path,
                total,
                format!("\x1b[32m{}\x1b[31m{}\x1b[0m", "+".repeat(*ins), "-".repeat(*del)),
                width = max_path_len
            );
        }
    }

    if !stats.is_empty() {
        println!(
            " {} file(s) changed, {} insertion(s)(+), {} deletion(s)(-)",
            stats.len(),
            total_insertions,
            total_deletions
        );
    }
}
