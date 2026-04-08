// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::Result;
use forge_core::object::snapshot::Author;
use forge_core::workspace::Workspace;

pub fn run(url: String, path: Option<String>, repo: Option<String>) -> Result<()> {
    // Derive repo name from URL path or use explicit --repo.
    let repo_name = repo.unwrap_or_else(|| "default".into());

    // Derive directory name: explicit --path, or repo name.
    let dir_name = path.unwrap_or_else(|| repo_name.clone());

    let target = std::env::current_dir()?.join(&dir_name);
    if target.exists() && std::fs::read_dir(&target)?.next().is_some() {
        anyhow::bail!("destination path '{}' already exists and is not empty", dir_name);
    }
    std::fs::create_dir_all(&target)?;

    println!("Cloning into '{}'...", target.display());

    // Initialize workspace.
    let author = Author {
        name: whoami::fallible::realname().unwrap_or_else(|_| "Unknown".into()),
        email: String::new(),
    };
    let ws = Workspace::init(&target, author)?;

    // Configure remote and repo name.
    let mut config = ws.config()?;
    config.add_remote("origin".into(), url)?;
    config.repo = repo_name;
    ws.save_config(&config)?;

    // Write default .forgeignore.
    let ignore_path = target.join(".forgeignore");
    if !ignore_path.exists() {
        std::fs::write(&ignore_path, forge_ignore::ForgeIgnore::default_content())?;
    }

    // Pull using the workspace we just created (not cwd).
    super::pull::run_with_workspace(&ws)?;

    println!("Clone complete.");
    Ok(())
}
