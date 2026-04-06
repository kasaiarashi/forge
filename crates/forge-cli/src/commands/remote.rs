// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{bail, Result};
use forge_core::workspace::Workspace;

pub fn run(action: Option<String>, args: Vec<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let mut config = ws.config()?;

    match action.as_deref() {
        None | Some("list") => {
            // List remotes.
            if config.remotes.is_empty() {
                println!("No remotes configured.");
                println!("  Use: forge remote add <name> <url>");
            } else {
                for remote in &config.remotes {
                    println!("  {} \t{}", remote.name, remote.url);
                }
            }
        }
        Some("add") => {
            if args.len() < 2 {
                bail!("Usage: forge remote add <name> <url>");
            }
            let name = &args[0];
            let url = &args[1];
            config.add_remote(name.clone(), url.clone())?;
            ws.save_config(&config)?;
            println!("Added remote '{}' -> {}", name, url);
        }
        Some("remove") => {
            if args.is_empty() {
                bail!("Usage: forge remote remove <name>");
            }
            let name = &args[0];
            config.remove_remote(name)?;
            ws.save_config(&config)?;
            println!("Removed remote '{}'", name);
        }
        Some("rename") => {
            if args.len() < 2 {
                bail!("Usage: forge remote rename <old> <new>");
            }
            config.rename_remote(&args[0], &args[1])?;
            ws.save_config(&config)?;
            println!("Renamed remote '{}' -> '{}'", args[0], args[1]);
        }
        Some("set-url") => {
            if args.len() < 2 {
                bail!("Usage: forge remote set-url <name> <url>");
            }
            config.set_remote_url(&args[0], args[1].clone())?;
            ws.save_config(&config)?;
            println!("Updated remote '{}' -> {}", args[0], args[1]);
        }
        Some(other) => {
            bail!(
                "Unknown remote action '{}'. Use: add, remove, rename, set-url, or list",
                other
            );
        }
    }

    Ok(())
}
