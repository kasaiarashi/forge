use anyhow::Result;
use forge_core::workspace::Workspace;

pub fn run(patterns: Vec<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let ignore_path = ws.root.join(".forgeignore");

    if patterns.is_empty() {
        // Show current patterns.
        if ignore_path.exists() {
            let content = std::fs::read_to_string(&ignore_path)?;
            print!("{}", content);
        } else {
            println!("No .forgeignore file found.");
        }
    } else {
        // Append patterns.
        let mut content = if ignore_path.exists() {
            std::fs::read_to_string(&ignore_path)?
        } else {
            String::new()
        };

        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        for pattern in &patterns {
            content.push_str(pattern);
            content.push('\n');
            println!("  added pattern: {}", pattern);
        }
        std::fs::write(&ignore_path, content)?;
    }

    Ok(())
}
