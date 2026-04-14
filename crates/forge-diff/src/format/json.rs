//! JSON output for `forge diff --json` — machine-readable unified diff.

use similar::ChangeTag;

use super::file_diff::FileDiff;
use crate::asset_paths::is_ue_companion_path;

pub fn format_json(diffs: &[FileDiff], out: &mut String) -> Result<(), serde_json::Error> {
    let mut entries = Vec::new();

    for diff in diffs {
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

    out.push_str(&serde_json::to_string_pretty(&entries)?);
    out.push('\n');
    Ok(())
}
