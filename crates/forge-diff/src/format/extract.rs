//! `--extract` mode: write old/new versions of diffed files to temp files and
//! print the paths, enabling external diff tools (e.g. the UE editor's
//! built-in diff viewer) to compare them:
//! `UE4Editor.exe -diff <left_path> <right_path>`

use std::io;
use std::path::PathBuf;

use super::file_diff::FileDiff;

/// Writes old/new files to `std::env::temp_dir() / "forge-diff-extract"` and
/// prints human-readable summaries to stdout. Returns the total extracted
/// file count on success.
pub fn print_extract(diffs: &[FileDiff]) -> io::Result<usize> {
    if diffs.is_empty() {
        println!("No differences found.");
        return Ok(0);
    }

    let temp_dir = std::env::temp_dir().join("forge-diff-extract");
    std::fs::create_dir_all(&temp_dir)?;

    for diff in diffs {
        let file_name = std::path::Path::new(&diff.path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();

        let old_path: Option<PathBuf> = if !diff.old_content.is_empty() {
            let p = temp_dir.join(format!("old_{}", file_name));
            std::fs::write(&p, &diff.old_content)?;
            Some(p)
        } else {
            None
        };

        let new_path: Option<PathBuf> = if !diff.new_content.is_empty() {
            let p = temp_dir.join(format!("new_{}", file_name));
            std::fs::write(&p, &diff.new_content)?;
            Some(p)
        } else {
            None
        };

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
    Ok(diffs.len())
}
