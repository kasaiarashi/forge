use anyhow::Result;
use forge_core::workspace::{HeadRef, Workspace};

pub fn run(name: String) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    // Verify branch exists.
    let _tip = ws.get_branch_tip(&name)?;

    // TODO: verify clean working tree, diff trees, update files.

    ws.set_head(&HeadRef::Branch(name.clone()))?;
    println!("Switched to branch '{}'", name);

    Ok(())
}
