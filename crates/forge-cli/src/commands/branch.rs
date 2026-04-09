use anyhow::{bail, Result};
use forge_core::workspace::{HeadRef, Workspace};

pub fn run(name: Option<String>, delete: bool, json: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    match name {
        None => {
            // List branches.
            let current = ws.current_branch()?;
            let branches = ws.list_branches()?;
            if json {
                let out = serde_json::json!({
                    "current": current.as_deref().unwrap_or(""),
                    "branches": branches,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                for branch in &branches {
                    if current.as_deref() == Some(branch.as_str()) {
                        println!("* \x1b[32m{}\x1b[0m", branch);
                    } else {
                        println!("  {}", branch);
                    }
                }
            }
        }
        Some(name) if delete => {
            let current = ws.current_branch()?;
            if current.as_deref() == Some(name.as_str()) {
                bail!("Cannot delete the current branch");
            }
            let ref_path = ws.forge_dir().join("refs").join("heads").join(&name);
            if !ref_path.exists() {
                bail!("Branch '{}' not found", name);
            }
            std::fs::remove_file(&ref_path)?;
            println!("Deleted branch '{}'", name);
        }
        Some(name) => {
            // Create branch at current HEAD.
            let ref_path = ws.forge_dir().join("refs").join("heads").join(&name);
            if ref_path.exists() {
                bail!("Branch '{}' already exists", name);
            }
            let head = ws.head_snapshot()?;
            ws.set_branch_tip(&name, &head)?;
            println!("Created branch '{}' at {}", name, head.short());

            // In detached HEAD, `forge branch` only creates the ref —
            // it does NOT switch to it. Without the hint, users hit a
            // nasty footgun: their next commit lands on (advanced)
            // detached HEAD and the freshly-created branch stays at
            // the old position. Flag it explicitly.
            if matches!(ws.head()?, HeadRef::Detached(_)) {
                println!();
                println!(
                    "note: HEAD is still detached — new commits will NOT land on '{name}' yet."
                );
                println!("      To start committing to '{name}', run:");
                println!("          forge switch {name}");
                println!("      Or combine both steps next time with:");
                println!("          forge switch -c {name}");
            }
        }
    }

    Ok(())
}
