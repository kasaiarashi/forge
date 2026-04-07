use anyhow::Result;
use forge_core::diff::{diff_maps, flatten_tree, DiffEntry};
use forge_core::hash::ForgeHash;
use forge_core::workspace::Workspace;
use std::collections::HashMap;

pub fn run(count: u32, file: Option<String>, json: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    // Build a map of commit hash -> list of branch names pointing to it.
    let head_hash = ws.head_snapshot()?;
    let current_branch = ws.current_branch()?;
    let branches = ws.list_branches()?;

    let mut ref_map: HashMap<ForgeHash, Vec<String>> = HashMap::new();
    for branch in &branches {
        if let Ok(tip) = ws.get_branch_tip(branch) {
            ref_map.entry(tip).or_default().push(branch.clone());
        }
    }

    // Normalize file filter path.
    let filter = file.map(|f| f.replace('\\', "/").trim_start_matches("./").to_string());

    let mut current = head_hash;
    let mut shown = 0u32;
    let mut json_entries = Vec::new();

    while !current.is_zero() && shown < count {
        let snapshot = ws.object_store.get_snapshot(&current)?;

        // If filtering by file, check if this commit touches it.
        if let Some(ref filter_path) = filter {
            let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();
            let current_flat = ws
                .object_store
                .get_tree(&snapshot.tree)
                .ok()
                .map(|t| flatten_tree(&t, "", &get_tree))
                .unwrap_or_default();

            let parent_flat = snapshot
                .parents
                .first()
                .filter(|h| !h.is_zero())
                .and_then(|h| ws.object_store.get_snapshot(h).ok())
                .and_then(|ps| ws.object_store.get_tree(&ps.tree).ok())
                .map(|t| flatten_tree(&t, "", &get_tree))
                .unwrap_or_default();

            let changes = diff_maps(&parent_flat, &current_flat);
            let touches_file = changes.iter().any(|d| {
                let path = match d {
                    DiffEntry::Added { path, .. }
                    | DiffEntry::Deleted { path, .. }
                    | DiffEntry::Modified { path, .. } => path,
                };
                path == filter_path || path.starts_with(&format!("{}/", filter_path))
            });

            if !touches_file {
                current = snapshot.parents.first().copied().unwrap_or(ForgeHash::ZERO);
                continue;
            }
        }

        if json {
            json_entries.push(serde_json::json!({
                "hash": current.to_hex(),
                "short_hash": current.short(),
                "author": {
                    "name": snapshot.author.name,
                    "email": snapshot.author.email,
                },
                "date": snapshot.timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                "message": snapshot.message,
            }));
        } else {
            // Build decoration string like git: (HEAD -> dev, master)
            let decorations = if let Some(refs) = ref_map.get(&current) {
                let mut parts: Vec<String> = Vec::new();
                for branch in refs {
                    if current_branch.as_deref() == Some(branch.as_str()) {
                        parts.insert(
                            0,
                            format!("\x1b[1;36mHEAD -> \x1b[1;32m{}\x1b[0m", branch),
                        );
                    } else {
                        parts.push(format!("\x1b[1;32m{}\x1b[0m", branch));
                    }
                }
                if parts.is_empty() {
                    String::new()
                } else {
                    format!(
                        " \x1b[33m(\x1b[0m{}\x1b[33m)\x1b[0m",
                        parts.join("\x1b[33m, \x1b[0m")
                    )
                }
            } else if current == head_hash && current_branch.is_none() {
                " \x1b[33m(\x1b[1;36mHEAD\x1b[33m)\x1b[0m".to_string()
            } else {
                String::new()
            };

            println!(
                "\x1b[33mcommit {}\x1b[0m{}",
                current.short(),
                decorations
            );
            println!(
                "Author: {} <{}>",
                snapshot.author.name, snapshot.author.email
            );
            println!(
                "Date:   {}",
                snapshot.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
            );
            println!();
            println!("    {}", snapshot.message);
            println!();
        }

        current = snapshot.parents.first().copied().unwrap_or(ForgeHash::ZERO);
        shown += 1;
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&json_entries)?);
    } else if shown == 0 {
        println!("No commits yet.");
    }

    Ok(())
}
