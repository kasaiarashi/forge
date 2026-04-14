//! `--stat` shortstat-equivalent output.

use std::fmt::Write as _;

use similar::ChangeTag;

use super::file_diff::FileDiff;
use crate::asset_paths::is_ue_companion_path;

pub fn format_stat(diffs: &[FileDiff], out: &mut String) {
    let mut total_insertions = 0usize;
    let mut total_deletions = 0usize;
    let mut max_path_len = 0usize;

    let mut stats: Vec<(&str, &str, usize, usize)> = Vec::new();
    for diff in diffs {
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

    for (path, status, ins, del) in &stats {
        if *status == "Bin" {
            let _ = writeln!(out, " {:<width$} | Bin", path, width = max_path_len);
        } else {
            let total = ins + del;
            let _ = writeln!(
                out,
                " {:<width$} | {:>4} {}",
                path,
                total,
                format!("\x1b[32m{}\x1b[31m{}\x1b[0m", "+".repeat(*ins), "-".repeat(*del)),
                width = max_path_len
            );
        }
    }

    if !stats.is_empty() {
        let _ = writeln!(
            out,
            " {} file(s) changed, {} insertion(s)(+), {} deletion(s)(-)",
            stats.len(),
            total_insertions,
            total_deletions
        );
    }
}
