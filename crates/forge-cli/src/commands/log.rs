use anyhow::Result;
use forge_core::workspace::Workspace;

pub fn run(count: u32, _file: Option<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    let mut current = ws.head_snapshot()?;
    let mut shown = 0u32;

    while !current.is_zero() && shown < count {
        let snapshot = ws.object_store.get_snapshot(&current)?;

        println!(
            "\x1b[33msnapshot {}\x1b[0m",
            current.short()
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

        current = snapshot.parents.first().copied().unwrap_or(forge_core::hash::ForgeHash::ZERO);
        shown += 1;
    }

    if shown == 0 {
        println!("No snapshots yet.");
    }

    Ok(())
}
