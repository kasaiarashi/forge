use anyhow::Result;
use forge_core::workspace::Workspace;

pub fn run(_snapshot: Option<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let _ws = Workspace::discover(&cwd)?;

    // TODO: implement tree diff + file content diff
    println!("diff: not yet implemented");
    Ok(())
}
