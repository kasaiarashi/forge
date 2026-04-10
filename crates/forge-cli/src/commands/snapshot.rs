use anyhow::{bail, Result};
use chrono::Utc;
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::object::snapshot::{Author, Snapshot};
use forge_core::object::tree::{EntryKind, Tree, TreeEntry};
use forge_core::workspace::{HeadRef, Workspace, WorkspaceConfig};
use std::collections::BTreeMap;

use crate::credentials;

pub fn run(message: Option<String>, all: bool, amend: bool, json: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let mut index = Index::load(&ws.forge_dir().join("index"))?;

    // If --all, auto-stage all modified/deleted files.
    if all {
        auto_stage(&ws, &mut index)?;
    }

    let staged: Vec<(String, forge_core::index::IndexEntry)> = index
        .entries
        .iter()
        .filter(|(_, e)| e.staged)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    // For a normal commit, we require staged content. For an amend, an empty
    // staging area is fine — that's a message-only or no-op rewrite.
    if !amend && staged.is_empty() {
        bail!("Nothing staged. Use `forge add` or `forge commit --all`.");
    }

    // Get the current tip — used as the parent for normal commits and as the
    // commit-being-rewritten for amends.
    let head_hash = ws.head_snapshot()?;

    // Resolve parents, message, author, and timestamp. The amend path copies
    // the old commit's parents (so the new commit slots into the *same*
    // history position), preserves its author + timestamp (forge has no
    // separate committer field, so refreshing would lose the original "when"
    // signal), and falls back to the old message when -m is omitted.
    let (parents, final_message, final_author, final_timestamp) = if amend {
        if head_hash.is_zero() {
            bail!("Nothing to amend; this repo has no commits yet");
        }
        let old = ws.object_store.get_snapshot(&head_hash)?;
        let msg = match message {
            Some(m) if !m.is_empty() => m,
            _ => old.message.clone(),
        };
        (old.parents.clone(), msg, old.author.clone(), old.timestamp)
    } else {
        let msg = message
            .filter(|m| !m.is_empty())
            .ok_or_else(|| anyhow::anyhow!("commit message is required (use -m <msg>)"))?;
        let parents = if head_hash.is_zero() {
            vec![]
        } else {
            vec![head_hash]
        };
        let config = ws.config()?;
        (parents, msg, resolve_commit_author(&config), Utc::now())
    };

    // Load previous snapshot's tree hash for incremental tree building. For
    // an amend we want to compare against the *grandparent*'s tree (i.e. the
    // first parent of the commit we're replacing) so build_tree can still
    // reuse unchanged subtrees from the actual history. For a normal commit,
    // it's just HEAD's tree.
    let prev_tree_hash = if amend {
        parents
            .first()
            .and_then(|p| ws.object_store.get_snapshot(p).ok())
            .map(|s| s.tree)
    } else if !head_hash.is_zero() {
        ws.object_store.get_snapshot(&head_hash).ok().map(|s| s.tree)
    } else {
        None
    };

    // Build tree hierarchy from all entries, excluding staged deletions (ZERO hash).
    let all_entries: BTreeMap<String, &IndexEntry> = index
        .entries
        .iter()
        .filter(|(_, v)| !v.hash.is_zero())
        .map(|(k, v)| (k.clone(), v))
        .collect();
    let root_tree = build_tree(&ws, &all_entries, prev_tree_hash.as_ref())?;
    let tree_hash = ws.object_store.put_tree(&root_tree)?;

    let snapshot = Snapshot {
        tree: tree_hash,
        parents,
        author: final_author,
        message: final_message.clone(),
        timestamp: final_timestamp,
        metadata: Default::default(),
    };

    let snap_hash = ws.object_store.put_snapshot(&snapshot)?;

    // Advance HEAD to the new snapshot.
    //
    //  * Branch HEAD: update the branch tip. HEAD is "ref: refs/heads/X",
    //    so moving the branch tip implicitly moves HEAD with it. For amend
    //    this overwrites the tip in place — exactly what we want.
    //  * Detached HEAD: rewrite HEAD itself so the new commit is
    //    reachable. Without this, the snapshot would go into the object
    //    store but nothing would reference it — a silent data-loss bug
    //    that orphans every commit made while detached.
    match ws.head()? {
        HeadRef::Branch(branch) => {
            ws.set_branch_tip(&branch, &snap_hash)?;
        }
        HeadRef::Detached(_) => {
            ws.set_head(&HeadRef::Detached(snap_hash))?;
        }
    }

    // Remove deleted entries (ZERO hash) and clear staged flags.
    index.entries.retain(|_, e| !e.hash.is_zero());
    index.clear_staged();
    index.save(&ws.forge_dir().join("index"))?;

    if json {
        let mut obj = serde_json::json!({
            "hash": snap_hash.to_hex(),
            "short_hash": snap_hash.short(),
            "message": final_message,
            "files": staged.len(),
        });
        if amend {
            obj["amended"] = serde_json::json!(true);
            obj["replaced"] = serde_json::json!(head_hash.to_hex());
        }
        println!("{}", serde_json::to_string_pretty(&obj)?);
    } else if amend {
        println!("Amended {} (was {})", snap_hash.short(), head_hash.short());
        println!("  {} file(s) | {}", staged.len(), final_message);
    } else {
        println!("Committed {}", snap_hash.short());
        println!("  {} file(s) | {}", staged.len(), final_message);
    }

    Ok(())
}

fn auto_stage(ws: &Workspace, index: &mut Index) -> Result<()> {
    let ignore = forge_ignore::ForgeIgnore::from_file(&ws.root.join(".forgeignore"))
        .unwrap_or_default();

    // Check existing entries for modifications.
    let paths: Vec<String> = index.entries.keys().cloned().collect();
    for path in &paths {
        let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if !abs_path.exists() {
            if let Some(entry) = index.entries.get_mut(path) {
                entry.staged = true;
            }
            continue;
        }

        let data = std::fs::read(&abs_path)?;
        let hash = ForgeHash::from_bytes(&data);
        if let Some(entry) = index.entries.get(path) {
            if hash != entry.hash {
                // File modified — re-add it.
                crate::commands::add::run(vec![path.clone()])?;
            }
        }
    }

    // Also add untracked files.
    for entry in walkdir::WalkDir::new(&ws.root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(&ws.root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");

            if rel.starts_with(".forge/") || rel.starts_with(".forge\\") {
                continue;
            }
            if ignore.is_ignored(&rel) {
                continue;
            }
            if !index.entries.contains_key(&rel) {
                crate::commands::add::run(vec![rel])?;
                // Reload index since add modifies it.
                *index = Index::load(&ws.forge_dir().join("index"))?;
            }
        }
    }

    Ok(())
}

/// Build a Tree hierarchy from all index entries, reusing unchanged subtrees
/// from the previous commit to avoid redundant serialization and storage.
fn build_tree(
    ws: &Workspace,
    entries: &BTreeMap<String, &IndexEntry>,
    prev_tree_hash: Option<&ForgeHash>,
) -> Result<Tree> {
    // Group entries by top-level directory component.
    let mut dirs: BTreeMap<String, BTreeMap<String, &IndexEntry>> = BTreeMap::new();
    let mut files: Vec<TreeEntry> = Vec::new();

    for (path, entry) in entries {
        if let Some(slash_pos) = path.find('/') {
            let dir_name = &path[..slash_pos];
            let rest = &path[slash_pos + 1..];
            dirs.entry(dir_name.to_string())
                .or_default()
                .insert(rest.to_string(), entry);
        } else {
            files.push(TreeEntry {
                name: path.clone(),
                kind: EntryKind::File,
                hash: entry.object_hash,
                size: entry.size,
            });
        }
    }

    // Load previous tree for comparison (if available).
    let prev_tree = prev_tree_hash.and_then(|h| ws.object_store.get_tree(h).ok());
    let prev_map: std::collections::HashMap<String, &TreeEntry> = prev_tree
        .as_ref()
        .map(|t| t.entries.iter().map(|e| (e.name.clone(), e)).collect())
        .unwrap_or_default();

    // Build subtrees, reusing unchanged ones via content-addressed hashing.
    for (dir_name, sub_entries) in &dirs {
        let prev_dir_hash = prev_map
            .get(dir_name)
            .filter(|e| e.kind == EntryKind::Directory)
            .map(|e| &e.hash);
        let subtree = build_tree(ws, sub_entries, prev_dir_hash)?;
        let subtree_hash = ws.object_store.put_tree(&subtree)?;
        files.push(TreeEntry {
            name: dir_name.clone(),
            kind: EntryKind::Directory,
            hash: subtree_hash,
            size: 0,
        });
    }

    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Tree { entries: files })
}

/// Resolve the commit author.
///
/// Priority is:
///
/// 1. Stored credential for the workspace's default remote (set by
///    `forge login` from the WhoAmI response). The PAT's identity is the
///    canonical "who is committing here" — overriding whatever the OS user
///    happens to be — so a logged-in machine always commits as the
///    authenticated forge user, not the local Windows username.
/// 2. Workspace `user.name` / `user.email` from `.forge/config.json`. This
///    is what `forge init` pre-fills from `whoami` and is the offline /
///    not-logged-in fallback.
fn resolve_commit_author(config: &WorkspaceConfig) -> Author {
    let mut author = config.user.clone();
    if let Some(server_url) = config.default_remote_url() {
        if let Ok(Some(cred)) = credentials::load(server_url) {
            // Credential wins when set. display_name is preferred over the
            // raw username for the human-readable name field.
            if !cred.display_name.is_empty() {
                author.name = cred.display_name;
            } else if !cred.user.is_empty() {
                author.name = cred.user;
            }
            if !cred.email.is_empty() {
                author.email = cred.email;
            }
        }
    }
    author
}
