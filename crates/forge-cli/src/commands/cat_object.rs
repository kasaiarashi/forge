use anyhow::{bail, Result};
use forge_core::hash::ForgeHash;
use forge_core::object::tree::EntryKind;
use forge_core::workspace::Workspace;

pub fn run(object: String) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    let hash = ws.resolve_ref(&object).or_else(|_| {
        ForgeHash::from_hex(&object)
    })?;

    // Read raw data to determine object type from the prefix byte.
    let data = ws.object_store.chunks.get(&hash)
        .map_err(|_| anyhow::anyhow!("object not found: {}", hash.short()))?;

    if data.is_empty() {
        bail!("object is empty: {}", hash.short());
    }

    let type_byte = data[0];
    match type_byte {
        // Snapshot (4)
        4 => {
            let snap = ws.object_store.get_snapshot(&hash)?;
            println!("type: snapshot");
            println!("hash: {}", hash.to_hex());
            println!("tree: {}", snap.tree.to_hex());
            println!("parents:");
            for p in &snap.parents {
                println!("  {}", p.to_hex());
            }
            println!("author: {} <{}>", snap.author.name, snap.author.email);
            println!("date:   {}", snap.timestamp.format("%Y-%m-%d %H:%M:%S UTC"));
            println!("message: {}", snap.message);
        }
        // Tree (3)
        3 => {
            let tree = ws.object_store.get_tree(&hash)?;
            println!("type: tree");
            println!("hash: {}", hash.to_hex());
            println!("entries: ({})", tree.entries.len());
            for entry in &tree.entries {
                let kind = match entry.kind {
                    EntryKind::File => "file",
                    EntryKind::Directory => "dir ",
                    EntryKind::Symlink => "link",
                };
                println!("  {} {} {} {}", kind, entry.hash.short(), entry.size, entry.name);
            }
        }
        // ChunkedBlob (2)
        2 => {
            let chunked = ws.object_store.get_chunked_blob(&hash)?;
            println!("type: chunked-blob");
            println!("hash: {}", hash.to_hex());
            println!("total_size: {}", chunked.total_size);
            println!("chunks: ({})", chunked.chunks.len());
            for (i, chunk) in chunked.chunks.iter().enumerate() {
                println!("  [{}] {} offset={} size={}", i, chunk.hash.short(), chunk.offset, chunk.size);
            }
        }
        // Blob (1) or raw data
        _ => {
            println!("type: blob");
            println!("hash: {}", hash.to_hex());
            println!("size: {} bytes", data.len());
        }
    }

    Ok(())
}
