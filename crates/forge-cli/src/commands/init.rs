use anyhow::Result;
use forge_core::object::snapshot::Author;
use forge_core::workspace::Workspace;
use forge_ignore::ForgeIgnore;

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir()?;

    // TODO: prompt for user info or read from global config
    let author = Author {
        name: whoami::fallible::realname().unwrap_or_else(|_| "Unknown".into()),
        email: String::new(),
    };

    let ws = Workspace::init(&cwd, author)?;

    // Write default .forgeignore
    let ignore_path = cwd.join(".forgeignore");
    if !ignore_path.exists() {
        std::fs::write(&ignore_path, ForgeIgnore::default_content())?;
    }

    println!("Initialized forge workspace at {}", ws.root.display());
    Ok(())
}
