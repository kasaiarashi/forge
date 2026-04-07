use anyhow::{bail, Result};
use forge_core::diff::{diff_maps, flatten_tree, DiffEntry};
use forge_core::hash::ForgeHash;
use forge_core::workspace::Workspace;
use std::collections::BTreeMap;

pub fn run(commit: Option<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    let hash = match commit {
        Some(ref c) => ForgeHash::from_hex(c)?,
        None => ws.head_snapshot()?,
    };

    if hash.is_zero() {
        bail!("No commits yet.");
    }

    let snapshot = ws.object_store.get_snapshot(&hash)?;

    // Print commit header (like log).
    println!("\x1b[33mcommit {}\x1b[0m", hash.to_hex());
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

    // Compute diff against parent.
    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();

    let parent_map = if let Some(parent_hash) = snapshot.parents.first() {
        if !parent_hash.is_zero() {
            if let Ok(parent_snap) = ws.object_store.get_snapshot(parent_hash) {
                if let Some(parent_tree) = get_tree(&parent_snap.tree) {
                    flatten_tree(&parent_tree, "", &get_tree)
                } else {
                    BTreeMap::new()
                }
            } else {
                BTreeMap::new()
            }
        } else {
            BTreeMap::new()
        }
    } else {
        BTreeMap::new()
    };

    let current_map = if let Some(tree) = get_tree(&snapshot.tree) {
        flatten_tree(&tree, "", &get_tree)
    } else {
        BTreeMap::new()
    };

    let changes = diff_maps(&parent_map, &current_map);

    if changes.is_empty() {
        println!("(no changes)");
    } else {
        for change in &changes {
            match change {
                DiffEntry::Added { path, size, .. } => {
                    println!("\x1b[32mA\x1b[0m  {} ({} bytes)", path, size);
                }
                DiffEntry::Deleted { path, size, .. } => {
                    println!("\x1b[31mD\x1b[0m  {} ({} bytes)", path, size);
                }
                DiffEntry::Modified {
                    path,
                    old_size,
                    new_size,
                    ..
                } => {
                    let delta = *new_size as i64 - *old_size as i64;
                    let sign = if delta >= 0 { "+" } else { "" };
                    println!(
                        "\x1b[33mM\x1b[0m  {} ({} -> {} bytes, {}{})",
                        path, old_size, new_size, sign, delta
                    );
                }
            }
        }
    }

    Ok(())
}
