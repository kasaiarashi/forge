// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

use anyhow::{bail, Result};
use forge_core::workspace::{WorkflowMode, Workspace};

pub fn run(key: Option<String>, value: Option<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let mut config = ws.config()?;

    match (key.as_deref(), value.as_deref()) {
        (None, _) => {
            // Show all config.
            println!("user.name     = {}", config.user.name);
            println!("user.email    = {}", config.user.email);
            println!(
                "repo          = {}",
                if config.repo.is_empty() {
                    "default"
                } else {
                    &config.repo
                }
            );
            println!("workflow      = {}", config.workflow);
            println!("workspace_id  = {}", config.workspace_id);
            if !config.remotes.is_empty() {
                println!("remotes:");
                for r in &config.remotes {
                    println!("  {} = {}", r.name, r.url);
                }
            }
            if !config.auto_lock_patterns.is_empty() {
                println!("auto_lock     = {}", config.auto_lock_patterns.join(", "));
            }
        }
        (Some("workflow"), Some(val)) => {
            config.workflow = match val {
                "lock" => WorkflowMode::Lock,
                "merge" => WorkflowMode::Merge,
                _ => bail!("Invalid workflow mode '{}'. Use 'lock' or 'merge'.", val),
            };
            ws.save_config(&config)?;
            println!("Workflow set to '{}'", config.workflow);
            match config.workflow {
                WorkflowMode::Lock => {
                    println!("  Binary files must be locked before editing.");
                    println!("  Conflicts are prevented — only the lock holder can push changes.");
                }
                WorkflowMode::Merge => {
                    println!("  Anyone can edit freely. Conflicts are resolved at push time");
                    println!("  by diffing and choosing which version to keep.");
                }
            }
        }
        (Some("user.name"), Some(val)) => {
            config.user.name = val.to_string();
            ws.save_config(&config)?;
            println!("user.name = {}", val);
        }
        (Some("user.email"), Some(val)) => {
            config.user.email = val.to_string();
            ws.save_config(&config)?;
            println!("user.email = {}", val);
        }
        (Some("repo"), Some(val)) => {
            config.repo = val.to_string();
            ws.save_config(&config)?;
            println!("repo = {}", val);
        }
        (Some(key), None) => match key {
            "workflow" => println!("{}", config.workflow),
            "user.name" => println!("{}", config.user.name),
            "user.email" => println!("{}", config.user.email),
            "repo" => println!(
                "{}",
                if config.repo.is_empty() {
                    "default"
                } else {
                    &config.repo
                }
            ),
            "workspace_id" => println!("{}", config.workspace_id),
            _ => bail!(
                "Unknown config key '{}'. Known: workflow, user.name, user.email, repo",
                key
            ),
        },
        (Some(key), Some(_)) => {
            bail!(
                "Cannot set '{}'. Known writable keys: workflow, user.name, user.email, repo",
                key
            );
        }
    }

    Ok(())
}
