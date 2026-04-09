use anyhow::{bail, Result};
use forge_core::diff::flatten_tree;
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::workspace::{HeadRef, Workspace};
use std::time::SystemTime;

pub fn run(target: Option<String>, paths: Vec<String>) -> Result<()> {
    if paths.is_empty() {
        // No paths: move HEAD to a branch or a commit.
        let Some(target) = target else {
            bail!("Usage: forge checkout <branch|commit> or forge checkout [<commit>] -- <paths>");
        };

        let cwd = std::env::current_dir()?;
        let ws = Workspace::discover(&cwd)?;

        // Branch names take priority. If the target resolves to an
        // existing branch, `switch` handles the normal branch-update path
        // (including the "already on branch" short-circuit).
        if ws.get_branch_tip(&target).is_ok() {
            return crate::commands::switch::run(target);
        }

        // Otherwise, treat the arg as a commit — full hash, short hash,
        // or tag.
        let commit = ws.resolve_ref(&target).map_err(|e| {
            anyhow::anyhow!(
                "'{target}' is neither a known branch nor a resolvable commit: {e}"
            )
        })?;
        if commit.is_zero() {
            bail!("commit '{target}' resolves to the zero hash");
        }

        // If the commit happens to be the tip of an existing branch,
        // attach to that branch instead of going detached. This is almost
        // always what the user meant — typing `forge checkout <tip>` then
        // committing in detached mode silently orphans the new commit,
        // which is a nasty footgun for a version control system to ship
        // with. When multiple branches share the tip, we pick the
        // current branch if possible, otherwise the first match in
        // branch-list order (alphabetical), and note it in the output.
        let matching_branches: Vec<String> = ws
            .list_branches()?
            .into_iter()
            .filter(|b| ws.get_branch_tip(b).map(|t| t == commit).unwrap_or(false))
            .collect();

        if !matching_branches.is_empty() {
            let current = ws.current_branch().ok().flatten();
            let branch = current
                .as_ref()
                .filter(|c| matching_branches.contains(c))
                .cloned()
                .unwrap_or_else(|| matching_branches[0].clone());

            if matching_branches.len() > 1 {
                eprintln!(
                    "note: commit {} is the tip of {} branches ({}). Attaching to '{}'.",
                    commit.short(),
                    matching_branches.len(),
                    matching_branches.join(", "),
                    branch
                );
            } else {
                eprintln!(
                    "note: commit {} is the tip of branch '{}'. Attaching to it instead of going detached.",
                    commit.short(),
                    branch
                );
            }
            return crate::commands::switch::run(branch);
        }

        // True detached-HEAD checkout — the commit isn't a branch tip.
        crate::commands::switch::move_to_commit(
            &ws,
            commit,
            HeadRef::Detached(commit),
        )?;

        println!(
            "HEAD is now at {} (detached).",
            commit.short()
        );
        println!(
            "Warning: new commits made here will be lost unless you create a branch:"
        );
        println!("    forge branch <new-name>");
        println!("    forge switch <new-name>");
        return Ok(());
    }

    // Paths given: restore files from a commit (or HEAD).
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    let commit_hash = match &target {
        Some(ref_str) => ws.resolve_ref(ref_str)?,
        None => ws.head_snapshot()?,
    };

    if commit_hash.is_zero() {
        bail!("No commits to restore from.");
    }

    let snap = ws.object_store.get_snapshot(&commit_hash)?;
    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();
    let tree = ws.object_store.get_tree(&snap.tree)?;
    let file_map = flatten_tree(&tree, "", &get_tree);

    let mut index = Index::load(&ws.forge_dir().join("index"))?;
    let mut restored = 0usize;

    for path in &paths {
        // Normalize path separators.
        let normalized = path.replace('\\', "/");

        // Check for exact match or prefix match (directory restore).
        let matching: Vec<(String, ForgeHash, u64)> = file_map
            .iter()
            .filter(|(p, _)| {
                *p == &normalized || p.starts_with(&format!("{}/", normalized))
            })
            .map(|(p, (h, s))| (p.clone(), *h, *s))
            .collect();

        if matching.is_empty() {
            eprintln!("warning: path '{}' not found in commit {}", path, commit_hash.short());
            continue;
        }

        for (file_path, hash, size) in matching {
            let content = ws.object_store.get_blob_data(&hash)?;
            let abs_path = ws.root.join(file_path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if let Some(parent) = abs_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&abs_path, &content)?;

            let mtime = std::fs::metadata(&abs_path)?
                .modified()?
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default();

            index.set(
                file_path.clone(),
                IndexEntry {
                    hash: ForgeHash::from_bytes(&content),
                    size,
                    mtime_secs: mtime.as_secs() as i64,
                    mtime_nanos: mtime.subsec_nanos(),
                    staged: false,
                    is_chunked: false,
                    object_hash: hash,
                },
            );

            restored += 1;
            println!("Restored '{}'", file_path);
        }
    }

    index.save(&ws.forge_dir().join("index"))?;

    if restored == 0 {
        bail!("No files restored.");
    }

    println!("Restored {} file(s) from {}", restored, commit_hash.short());
    Ok(())
}
