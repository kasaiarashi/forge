// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::Result;
use forge_core::object::snapshot::Author;
use forge_core::workspace::Workspace;

pub fn run(url: String, path: Option<String>) -> Result<()> {
    // Derive directory name from URL if not specified.
    let dir_name = path.unwrap_or_else(|| {
        url.rsplit('/')
            .next()
            .unwrap_or("forge-project")
            .trim_end_matches('/')
            .to_string()
    });

    let target = std::env::current_dir()?.join(&dir_name);
    std::fs::create_dir_all(&target)?;

    println!("Cloning into '{}'...", target.display());

    // Initialize workspace.
    let author = Author {
        name: whoami::fallible::realname().unwrap_or_else(|_| "Unknown".into()),
        email: String::new(),
    };
    let ws = Workspace::init(&target, author)?;

    // Set server URL in config.
    let mut config = ws.config()?;
    config.server_url = Some(url);
    let config_json = serde_json::to_string_pretty(&config)?;
    std::fs::write(ws.forge_dir().join("config.json"), config_json)?;

    // Write default .forgeignore.
    let ignore_path = target.join(".forgeignore");
    if !ignore_path.exists() {
        std::fs::write(&ignore_path, forge_ignore::ForgeIgnore::default_content())?;
    }

    // Pull from remote.
    super::pull::run()?;

    println!("Clone complete.");
    Ok(())
}
