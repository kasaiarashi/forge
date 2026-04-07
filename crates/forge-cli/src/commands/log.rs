use anyhow::Result;
use forge_core::hash::ForgeHash;
use forge_core::workspace::Workspace;
use std::collections::HashMap;

pub fn run(count: u32, _file: Option<String>) -> Result<()> {
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

    let mut current = head_hash;
    let mut shown = 0u32;

    while !current.is_zero() && shown < count {
        let snapshot = ws.object_store.get_snapshot(&current)?;

        // Build decoration string like git: (HEAD -> dev, master)
        let decorations = if let Some(refs) = ref_map.get(&current) {
            let mut parts: Vec<String> = Vec::new();
            for branch in refs {
                if current_branch.as_deref() == Some(branch.as_str()) {
                    parts.insert(0, format!(
                        "\x1b[1;36mHEAD -> \x1b[1;32m{}\x1b[0m", branch
                    ));
                } else {
                    parts.push(format!("\x1b[1;32m{}\x1b[0m", branch));
                }
            }
            if parts.is_empty() {
                String::new()
            } else {
                format!(" \x1b[33m(\x1b[0m{}\x1b[33m)\x1b[0m", parts.join("\x1b[33m, \x1b[0m"))
            }
        } else if current == head_hash && current_branch.is_none() {
            // Detached HEAD
            " \x1b[33m(\x1b[1;36mHEAD\x1b[33m)\x1b[0m".to_string()
        } else {
            String::new()
        };

        println!(
            "\x1b[33mcommit {}\x1b[0m{}",
            current.short(), decorations
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

        current = snapshot.parents.first().copied().unwrap_or(ForgeHash::ZERO);
        shown += 1;
    }

    if shown == 0 {
        println!("No commits yet.");
    }

    Ok(())
}
