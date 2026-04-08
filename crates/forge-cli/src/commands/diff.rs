use anyhow::{bail, Result};
use forge_core::diff::{diff_maps, flatten_tree, DiffEntry};
use forge_core::hash::ForgeHash;
use forge_core::index::Index;
use forge_core::workspace::Workspace;
use similar::ChangeTag;
use std::collections::{BTreeMap, HashMap};
use std::time::SystemTime;

pub fn run(commit: Option<String>, staged: bool, stat: bool, extract: bool, paths: Vec<String>, json: bool) -> Result<()> {
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

    if extract {
        print_extract(&file_diffs)?;
    } else if json {
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
    // Collect companion .uexp data for UE asset diffs.
    let uexp_map: HashMap<String, &[u8]> = diffs
        .iter()
        .filter(|d| is_ue_companion_path(&d.path) && d.path.to_lowercase().ends_with(".uexp"))
        .filter_map(|d| {
            let header_path = forge_core::asset_group::header_for_companion(&d.path)?;
            Some((header_path, d.new_content.as_slice()))
        })
        .collect();

    for diff in diffs {
        // Suppress standalone companion file entries — their changes are shown
        // as part of the parent .uasset diff.
        if is_ue_companion_path(&diff.path) {
            if let Some(header) = forge_core::asset_group::header_for_companion(&diff.path) {
                if diffs.iter().any(|d| d.path == header) {
                    continue; // Will be shown with the header file.
                }
            }
        }

        if diff.binary {
            // Try structured diff for UE assets.
            if is_uasset_path(&diff.path)
                && !diff.old_content.is_empty()
                && !diff.new_content.is_empty()
            {
                // Look up companion .uexp data for this header.
                let new_uexp = uexp_map.get(&diff.path).copied();
                if let Some(output) = try_structured_asset_diff_with_uexp(
                    &diff.path,
                    &diff.old_content,
                    None, // TODO: old uexp from object store
                    &diff.new_content,
                    new_uexp,
                ) {
                    print!("{}", output);
                    continue;
                }
            }
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
            let Some(first_op) = group.first() else { continue };
            let Some(last_op) = group.last() else { continue };
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
        // Suppress standalone companion files in JSON output too.
        if is_ue_companion_path(&diff.path) {
            if let Some(header) = forge_core::asset_group::header_for_companion(&diff.path) {
                if diffs.iter().any(|d| d.path == header) {
                    continue;
                }
            }
        }

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
            let Some(first_op) = group.first() else { continue };
            let Some(last_op) = group.last() else { continue };
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
        // Suppress standalone companion files in stat output.
        if is_ue_companion_path(&diff.path) {
            if let Some(header) = forge_core::asset_group::header_for_companion(&diff.path) {
                if diffs.iter().any(|d| d.path == header) {
                    continue;
                }
            }
        }

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

/// Extract old and new versions of diffed files to temp files.
///
/// Writes each version to a temp file and prints the paths, enabling external
/// diff tools (e.g., UE editor's built-in diff viewer) to compare them:
///   `UE4Editor.exe -diff <left_path> <right_path>`
fn print_extract(diffs: &[FileDiff]) -> Result<()> {
    if diffs.is_empty() {
        println!("No differences found.");
        return Ok(());
    }

    let temp_dir = std::env::temp_dir().join("forge-diff-extract");
    std::fs::create_dir_all(&temp_dir)?;

    let mut entries = Vec::new();

    for diff in diffs {
        let file_name = std::path::Path::new(&diff.path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();

        let old_path = if !diff.old_content.is_empty() {
            let p = temp_dir.join(format!("old_{}", file_name));
            std::fs::write(&p, &diff.old_content)?;
            Some(p)
        } else {
            None
        };

        let new_path = if !diff.new_content.is_empty() {
            let p = temp_dir.join(format!("new_{}", file_name));
            std::fs::write(&p, &diff.new_content)?;
            Some(p)
        } else {
            None
        };

        entries.push(serde_json::json!({
            "path": diff.path,
            "status": diff.status,
            "old_file": old_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "new_file": new_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        }));

        // Also print human-readable output.
        match diff.status {
            "added" => {
                println!("{} (added)", diff.path);
                if let Some(p) = &new_path {
                    println!("  new: {}", p.display());
                }
            }
            "deleted" => {
                println!("{} (deleted)", diff.path);
                if let Some(p) = &old_path {
                    println!("  old: {}", p.display());
                }
            }
            _ => {
                println!("{} ({})", diff.path, diff.status);
                if let Some(p) = &old_path {
                    println!("  old: {}", p.display());
                }
                if let Some(p) = &new_path {
                    println!("  new: {}", p.display());
                }
            }
        }
    }

    println!("\nExtracted {} file(s) to {}", diffs.len(), temp_dir.display());
    Ok(())
}

/// Check if a file path is a UE asset header that supports structured diffing.
fn is_uasset_path(path: &str) -> bool {
    forge_core::asset_group::is_header_path(path)
}

/// Check if a file path is a UE companion file (.uexp, .ubulk, .uptnl).
fn is_ue_companion_path(path: &str) -> bool {
    forge_core::asset_group::is_companion_path(path)
}

/// Attempt a structured diff with optional .uexp companion data.
fn try_structured_asset_diff_with_uexp(
    path: &str,
    old_data: &[u8],
    old_uexp: Option<&[u8]>,
    new_data: &[u8],
    new_uexp: Option<&[u8]>,
) -> Option<String> {
    use forge_core::uasset_diff::{self, parse_structured_with_uexp};

    let old_asset = parse_structured_with_uexp(old_data, old_uexp).ok()?;
    let new_asset = parse_structured_with_uexp(new_data, new_uexp).ok()?;

    let changes = uasset_diff::diff_assets_with_data(
        &old_asset, Some(old_data),
        &new_asset, Some(new_data),
    );

    if changes.is_empty() {
        return None; // No semantic changes detected.
    }

    // Build lookup maps for outer (parent) names.
    let new_outer: HashMap<String, Option<String>> = new_asset.exports.iter()
        .map(|e| (e.object_name.clone(), e.outer_name.clone()))
        .collect();
    let old_outer: HashMap<String, Option<String>> = old_asset.exports.iter()
        .map(|e| (e.object_name.clone(), e.outer_name.clone()))
        .collect();
    let new_import_outer: HashMap<String, Option<String>> = new_asset.imports.iter()
        .map(|i| (i.object_name.clone(), i.outer_name.clone()))
        .collect();
    let old_import_outer: HashMap<String, Option<String>> = old_asset.imports.iter()
        .map(|i| (i.object_name.clone(), i.outer_name.clone()))
        .collect();

    let mut output = String::new();
    output.push_str(&format!(
        "\x1b[1mdiff --forge a/{} b/{}\x1b[0m\n",
        path, path
    ));
    output.push_str(&format!(
        "  \x1b[36m[asset]\x1b[0m Engine: {} | Exports: {} | Imports: {}\n",
        new_asset.engine_version,
        new_asset.exports.len(),
        new_asset.imports.len()
    ));

    if !new_asset.parse_warnings.is_empty() {
        for w in &new_asset.parse_warnings {
            output.push_str(&format!("  \x1b[33mwarning: {}\x1b[0m\n", w));
        }
    }

    // Separate changes into categories for hierarchical display.
    let mut import_adds: Vec<&forge_core::uasset_diff::ImportInfo> = Vec::new();
    let mut import_removes: Vec<&forge_core::uasset_diff::ImportInfo> = Vec::new();
    let mut export_adds: Vec<(String, String)> = Vec::new(); // (name, class)
    let mut export_removes: Vec<(String, String)> = Vec::new();
    let mut property_changes: Vec<&uasset_diff::AssetChange> = Vec::new();

    for change in &changes {
        match change {
            uasset_diff::AssetChange::ImportAdded(imp) => import_adds.push(imp),
            uasset_diff::AssetChange::ImportRemoved(imp) => import_removes.push(imp),
            uasset_diff::AssetChange::ExportAdded { name, class } => {
                export_adds.push((name.clone(), class.clone()));
            }
            uasset_diff::AssetChange::ExportRemoved { name, class } => {
                export_removes.push((name.clone(), class.clone()));
            }
            _ => property_changes.push(change),
        }
    }

    // --- Imports: group by package, combine adds/removes ---
    {
        // Merge adds and removes into a unified map keyed by outer.
        let mut import_groups: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();

        for imp in &import_removes {
            let outer = old_import_outer.get(&imp.object_name)
                .and_then(|o| o.clone()).unwrap_or_default();
            import_groups.entry(outer).or_default().push(
                format!("  \x1b[31m- {} ({})\x1b[0m", imp.object_name, imp.class_name)
            );
        }
        for imp in &import_adds {
            let outer = new_import_outer.get(&imp.object_name)
                .and_then(|o| o.clone()).unwrap_or_default();
            import_groups.entry(outer).or_default().push(
                format!("  \x1b[32m+ {} ({})\x1b[0m", imp.object_name, imp.class_name)
            );
        }

        for (outer, lines) in &import_groups {
            if outer.is_empty() || lines.len() == 1 {
                for line in lines {
                    output.push_str(&format!("  import:{}\n", line.trim_start()));
                }
            } else {
                output.push_str(&format!("  \x1b[36m[import: {}]\x1b[0m\n", outer));
                for line in lines {
                    output.push_str(&format!("  {}\n", line));
                }
            }
        }
    }

    // --- Exports + property changes: unified by export name ---
    // Collect all per-export changes into a single map.
    let mut export_changes: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();

    for (name, class) in &export_removes {
        export_changes.entry(name.clone()).or_default().push(
            format!("\x1b[31m- {} ({})\x1b[0m", name, class)
        );
    }
    for (name, class) in &export_adds {
        export_changes.entry(name.clone()).or_default().push(
            format!("\x1b[32m+ {} ({})\x1b[0m", name, class)
        );
    }
    for change in &property_changes {
        match change {
            uasset_diff::AssetChange::PropertyChanged {
                export_name, property_path, old_value, new_value,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[33m~ {}\x1b[0m: {} \x1b[33m->\x1b[0m {}", property_path, old_value, new_value)
                );
            }
            uasset_diff::AssetChange::PropertyAdded {
                export_name, property_name, value,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[32m+ {}\x1b[0m: {}", property_name, value)
                );
            }
            uasset_diff::AssetChange::PropertyRemoved {
                export_name, property_name, value,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[31m- {}\x1b[0m: {}", property_name, value)
                );
            }
            uasset_diff::AssetChange::ExportDataChanged {
                export_name, description,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[33m~ {}\x1b[0m", description)
                );
            }
            uasset_diff::AssetChange::FieldAdded {
                export_name, field,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[32m+ variable: {}\x1b[0m", field)
                );
            }
            uasset_diff::AssetChange::FieldRemoved {
                export_name, field,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[31m- variable: {}\x1b[0m", field)
                );
            }
            uasset_diff::AssetChange::EnumValueAdded {
                export_name, value_name, display_name,
            } => {
                let label = if let Some(dn) = display_name {
                    format!("{} ({})", value_name, dn)
                } else {
                    value_name.clone()
                };
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[32m+ enum: {}\x1b[0m", label)
                );
            }
            uasset_diff::AssetChange::EnumValueRemoved {
                export_name, value_name,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[31m- enum: {}\x1b[0m", value_name)
                );
            }
            _ => {}
        }
    }

    // Now build the tree display.
    // Combine the outer maps from old and new for full coverage.
    let mut all_outer: HashMap<String, Option<String>> = old_outer;
    for (k, v) in &new_outer {
        all_outer.entry(k.clone()).or_insert_with(|| v.clone());
    }

    // Determine which exports are "tree roots" for add/remove grouping.
    let changed_export_set: std::collections::HashSet<&str> = export_adds.iter()
        .map(|(n, _)| n.as_str())
        .chain(export_removes.iter().map(|(n, _)| n.as_str()))
        .collect();

    // Build parent→children for added/removed exports.
    let mut tree_children: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut tree_roots: Vec<String> = Vec::new();

    for name in changed_export_set.iter() {
        let parent = all_outer.get(*name)
            .and_then(|o| o.as_deref())
            .unwrap_or("");
        if parent.is_empty() || !changed_export_set.contains(parent) {
            tree_roots.push(name.to_string());
        } else {
            tree_children.entry(parent.to_string())
                .or_default()
                .push(name.to_string());
        }
    }

    // Group tree roots by their outer for context headers.
    let mut root_groups: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for name in &tree_roots {
        let parent = all_outer.get(name.as_str())
            .and_then(|o| o.clone())
            .unwrap_or_default();
        root_groups.entry(parent).or_default().push(name.clone());
    }

    // Collect property-only changes (exports that have property/data changes but weren't added/removed).
    let mut prop_only: Vec<String> = Vec::new();
    for name in export_changes.keys() {
        if !changed_export_set.contains(name.as_str()) {
            prop_only.push(name.clone());
        }
    }

    // Track which exports we've already displayed.
    let mut displayed: std::collections::HashSet<String> = std::collections::HashSet::new();

    // --- Display tree-grouped export adds/removes ---
    for (context, roots) in &root_groups {
        if !context.is_empty() {
            output.push_str(&format!("  \x1b[36m[{}]\x1b[0m", context));
            // If the context itself has property changes, show them on separate lines.
            if let Some(lines) = export_changes.get(context.as_str()) {
                if !changed_export_set.contains(context.as_str()) {
                    output.push('\n');
                    for line in lines {
                        output.push_str(&format!("    {}\n", line));
                    }
                    displayed.insert(context.clone());
                } else {
                    output.push('\n');
                }
            } else {
                output.push('\n');
            }
        }
        for root_name in roots {
            write_unified_tree_node(
                &mut output, root_name, &export_changes, &tree_children, &all_outer, 2,
            );
            displayed.insert(root_name.clone());
        }
    }

    // --- Display property-only changes (modified exports not yet shown) ---
    // Group these by outer.
    let mut prop_groups: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for name in &prop_only {
        if displayed.contains(name) {
            continue;
        }
        let parent = all_outer.get(name.as_str())
            .and_then(|o| o.clone())
            .unwrap_or_default();
        prop_groups.entry(parent).or_default().push(name.clone());
    }

    for (context, names) in &prop_groups {
        // If all names share a context and it hasn't been shown yet, show as group.
        let show_context = !context.is_empty();
        if show_context && names.len() > 1 {
            output.push_str(&format!("  \x1b[36m[{}]\x1b[0m\n", context));
            for name in names {
                if let Some(lines) = export_changes.get(name) {
                    for line in lines {
                        output.push_str(&format!("    \x1b[36m[{}]\x1b[0m {}\n", name, line));
                    }
                }
            }
        } else {
            for name in names {
                if let Some(lines) = export_changes.get(name) {
                    let label = if show_context { context.as_str() } else { name.as_str() };
                    if lines.len() == 1 {
                        output.push_str(&format!("  \x1b[36m[{}]\x1b[0m {}\n", label, lines[0]));
                    } else {
                        output.push_str(&format!("  \x1b[36m[{}]\x1b[0m\n", label));
                        for line in lines {
                            output.push_str(&format!("    {}\n", line));
                        }
                    }
                }
            }
        }
    }

    Some(output)
}

/// Write a unified tree node showing its add/remove line plus children recursively.
fn write_unified_tree_node(
    output: &mut String,
    name: &str,
    export_changes: &std::collections::BTreeMap<String, Vec<String>>,
    tree_children: &std::collections::BTreeMap<String, Vec<String>>,
    all_outer: &HashMap<String, Option<String>>,
    depth: usize,
) {
    let indent = "  ".repeat(depth);

    // Write this node's own change lines (add or remove).
    if let Some(lines) = export_changes.get(name) {
        for line in lines {
            output.push_str(&format!("{}{}\n", indent, line));
        }
    }

    // Write children recursively.
    if let Some(children) = tree_children.get(name) {
        let mut sorted = children.clone();
        sorted.sort();
        for child in &sorted {
            write_unified_tree_node(output, child, export_changes, tree_children, all_outer, depth + 1);
        }
    }
}



