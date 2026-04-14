//! Default colored output for `forge diff` — ANSI-escaped unified diff with
//! UE-asset structured diff fallback.

use std::collections::HashMap;
use std::fmt::Write as _;

use similar::ChangeTag;

use super::file_diff::FileDiff;
use super::unified::try_structured_asset_diff_with_uexp;
use crate::asset_paths::{is_uasset_path, is_ue_companion_path};

pub fn format_colored(diffs: &[FileDiff], out: &mut String, class_stats: bool) {
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
                    continue;
                }
            }
        }

        if diff.binary {
            if is_uasset_path(&diff.path)
                && !diff.old_content.is_empty()
                && !diff.new_content.is_empty()
            {
                let new_uexp = uexp_map.get(&diff.path).copied();
                if let Some(output) = try_structured_asset_diff_with_uexp(
                    &diff.path,
                    &diff.old_content,
                    None,
                    &diff.new_content,
                    new_uexp,
                    class_stats,
                ) {
                    out.push_str(&output);
                    continue;
                }
            }
            let _ = writeln!(
                out,
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

        let _ = writeln!(
            out,
            "\x1b[1mdiff --forge a/{} b/{}\x1b[0m",
            diff.path, diff.path
        );
        match diff.status {
            "added" => {
                let _ = writeln!(out, "\x1b[1m--- /dev/null\x1b[0m");
                let _ = writeln!(out, "\x1b[1m+++ b/{}\x1b[0m", diff.path);
            }
            "deleted" => {
                let _ = writeln!(out, "\x1b[1m--- a/{}\x1b[0m", diff.path);
                let _ = writeln!(out, "\x1b[1m+++ /dev/null\x1b[0m");
            }
            _ => {
                let _ = writeln!(out, "\x1b[1m--- a/{}\x1b[0m", diff.path);
                let _ = writeln!(out, "\x1b[1m+++ b/{}\x1b[0m", diff.path);
            }
        }

        for group in &ops {
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

            let _ = writeln!(
                out,
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
                        let _ = write!(out, "{}{}{}{}", color, prefix, value, reset);
                    } else {
                        let _ = writeln!(out, "{}{}{}{}", color, prefix, value, reset);
                    }
                }
            }
        }
    }
}
