use anyhow::{bail, Result};
use forge_core::hash::ForgeHash;
use forge_core::workspace::Workspace;

pub fn run(name: Option<String>, commit: Option<String>, delete: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let tags_dir = ws.forge_dir().join("refs").join("tags");

    match name {
        None => {
            // List tags.
            if tags_dir.exists() {
                let mut tags: Vec<String> = Vec::new();
                for entry in std::fs::read_dir(&tags_dir)? {
                    let entry = entry?;
                    if entry.file_type()?.is_file() {
                        if let Some(name) = entry.file_name().to_str() {
                            tags.push(name.to_string());
                        }
                    }
                }
                tags.sort();
                for tag in &tags {
                    println!("{}", tag);
                }
            }
        }
        Some(name) if delete => {
            let tag_path = tags_dir.join(&name);
            if !tag_path.exists() {
                bail!("Tag '{}' not found", name);
            }
            std::fs::remove_file(&tag_path)?;
            println!("Deleted tag '{}'", name);
        }
        Some(name) => {
            let tag_path = tags_dir.join(&name);
            if tag_path.exists() {
                bail!("Tag '{}' already exists", name);
            }

            let hash = match commit {
                Some(ref c) => ForgeHash::from_hex(c)?,
                None => ws.head_snapshot()?,
            };

            if hash.is_zero() {
                bail!("No commits to tag");
            }

            std::fs::create_dir_all(&tags_dir)?;
            std::fs::write(&tag_path, hash.to_hex())?;
            println!("Created tag '{}' at {}", name, hash.short());
        }
    }

    Ok(())
}
