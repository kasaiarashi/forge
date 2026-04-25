use anyhow::{bail, Result};
use forge_core::diff::{diff_maps, DiffEntry};
use forge_core::hash::ForgeHash;
use forge_core::index::{Index, IndexEntry};
use forge_core::object::tree::{EntryKind, Tree};
use forge_core::workspace::{HeadRef, Workspace};
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::SystemTime;

pub fn run(name: String) -> Result<()> {
    run_with_create(name, false)
}

/// Entry point used by `forge switch [-c] <name>`.
///
/// * `create == false` → normal branch switch; the branch must exist.
/// * `create == true`  → create the branch at the current HEAD, then
///   switch to it. Equivalent to `git switch -c` / `git checkout -b`.
///   Commonly used to "rescue" a detached HEAD that has one or more
///   new commits on it.
pub fn run_with_create(name: String, create: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    run_with_create_in(&cwd, name, create)
}

/// FFI-facing entry point — same as [`run_with_create`] but with an
/// explicit workspace anchor so the GUI / UE plugin don't have to
/// mutate process CWD.
pub fn run_with_create_in(cwd: &std::path::Path, name: String, create: bool) -> Result<()> {
    let ws = Workspace::discover(cwd)?;

    if create {
        // Refuse to clobber an existing branch — users can manage that
        // explicitly with `forge branch -d` + recreate, or a future
        // `branch -f` force-move. Matches git's default.
        if ws.get_branch_tip(&name).is_ok() {
            bail!(
                "branch '{name}' already exists — drop `-c` to switch to it, \
                 or delete it first with `forge branch -d {name}`"
            );
        }
        let head = ws.head_snapshot()?;
        if head.is_zero() {
            bail!(
                "cannot create branch '{name}' before the first commit"
            );
        }
        ws.set_branch_tip(&name, &head)?;
        println!("Created branch '{name}' at {}", head.short());
    }

    // Verify the (now possibly just-created) target branch exists. If it
    // doesn't, fall through to the DWIM path: a remote-tracking ref left
    // by `forge fetch` is enough to materialize a local branch on the fly.
    // Matches `git switch <branch>` when only `origin/<branch>` exists.
    let target_commit = match ws.get_branch_tip(&name) {
        Ok(h) => h,
        Err(_) => match try_create_from_remote_ref(&ws, &name)? {
            Some(h) => h,
            None => {
                bail!(
                    "branch '{name}' does not exist locally or on any remote-tracking ref. \
                     Run `forge fetch` if you expect it from the server."
                );
            }
        },
    };

    // Bail if already on that branch.
    if let Ok(HeadRef::Branch(current)) = ws.head() {
        if current == name {
            println!("Already on branch '{}'", name);
            return Ok(());
        }
    }

    move_to_commit(&ws, target_commit, HeadRef::Branch(name.clone()))?;
    println!("Switched to branch '{}'", name);
    Ok(())
}

/// Shared worker used by `switch` and `checkout` to update the working
/// tree, index, and `HEAD` to the given commit. The caller decides what
/// `HEAD` should point at afterward:
///
///   * [`HeadRef::Branch(name)`] — normal branch switch.
///   * [`HeadRef::Detached(hash)`] — `git checkout <sha>` style detached
///     HEAD. `forge checkout <commit>` uses this.
///
/// Does a dirty check first so we never clobber uncommitted changes.
/// Safe to call when the workspace is already at `target_commit`: the
/// diff is empty and we just rewrite `HEAD`.
pub(crate) fn move_to_commit(
    ws: &Workspace,
    target_commit: ForgeHash,
    new_head: HeadRef,
) -> Result<()> {
    let index_path = ws.forge_dir().join("index");

    // Dirty-check: compare index entries against working tree. Parallel
    // because UE projects routinely have tens of thousands of indexed
    // assets and a serial stat()+rehash loop dominates wall time once
    // anything in the tree has had its mtime touched (build artifacts,
    // engine touches, antivirus, etc.).
    eprintln!("Checking working tree...");
    let index = Index::load(&index_path)?;
    let entries: Vec<(&String, &IndexEntry)> = index.entries.iter().collect();
    let dirty_msg = "You have uncommitted changes; commit or stash them first.";
    entries.par_iter().try_for_each(|(path, entry)| -> Result<()> {
        if entry.staged {
            bail!(dirty_msg);
        }

        let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let metadata = match std::fs::metadata(&abs_path) {
            Ok(m) => m,
            Err(_) => bail!(dirty_msg),
        };

        let mtime = metadata
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();

        if mtime.as_secs() as i64 == entry.mtime_secs
            && mtime.subsec_nanos() == entry.mtime_nanos
            && metadata.len() == entry.size
        {
            return Ok(());
        }

        // Stat mismatch — confirm via content hash before declaring dirty.
        let data = std::fs::read(&abs_path)?;
        let hash = ForgeHash::from_bytes(&data);
        if hash != entry.hash {
            bail!(dirty_msg);
        }
        Ok(())
    })?;

    // Current HEAD snapshot.
    let current_commit = ws.head_snapshot()?;

    // Same commit — only the HEAD pointer is changing (e.g.
    // branch -> detached at the current tip). Skip the tree work.
    if target_commit == current_commit {
        ws.set_head(&new_head)?;
        return Ok(());
    }

    // The working tree is clean, so the index already mirrors the current
    // commit's tree. Reuse it instead of walking thousands of unchanged
    // tree objects out of the object store.
    let old_flat: BTreeMap<String, (ForgeHash, u64)> = index
        .entries
        .iter()
        .map(|(p, e)| (p.clone(), (e.object_hash, e.size)))
        .collect();

    eprintln!("Loading target tree...");
    let new_flat: BTreeMap<String, (ForgeHash, u64)> = if target_commit.is_zero() {
        BTreeMap::new()
    } else {
        let snap = ws.object_store.get_snapshot(&target_commit)?;
        let tree = ws.object_store.get_tree(&snap.tree)?;
        flatten_tree_parallel(ws, &tree, "")?
    };

    // Diff the two trees and apply just the changed files.
    let changes = diff_maps(&old_flat, &new_flat);
    let n_changed = changes.len();
    if n_changed > 0 {
        eprintln!("Applying {n_changed} changes...");
    }
    for change in &changes {
        match change {
            DiffEntry::Added { path, .. } | DiffEntry::Modified { path, .. } => {
                let (obj_hash, _size) = &new_flat[path];
                let content = read_blob_content(ws, obj_hash)?;
                let abs = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                clear_readonly(&abs);
                std::fs::write(&abs, &content)?;
            }
            DiffEntry::Deleted { path, .. } => {
                let abs = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
                if abs.exists() {
                    if let Err(e) = std::fs::remove_file(&abs) {
                        eprintln!("warning: could not remove '{}': {}", path, e);
                    }
                }
            }
        }
    }

    // Rebuild full index from target tree. Parallel because per-entry we
    // stat the working file and (for chunked entries) reassemble the blob
    // to compute its content hash — both I/O bound, both safe to run on
    // disjoint paths.
    eprintln!("Rebuilding index...");
    let new_entries: Vec<(String, IndexEntry)> = new_flat
        .par_iter()
        .map(|(path, (hash, size))| {
            let is_chunked = is_chunked_object(ws, hash);

            let abs_path = ws.root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
            let (mtime_secs, mtime_nanos) = if abs_path.exists() {
                mtime_of(&abs_path)
            } else {
                (0, 0)
            };

            let final_content_hash = if is_chunked {
                match read_blob_content(ws, hash) {
                    Ok(data) => ForgeHash::from_bytes(&data),
                    Err(_) => ForgeHash::ZERO,
                }
            } else {
                *hash
            };

            (
                path.clone(),
                IndexEntry {
                    hash: final_content_hash,
                    size: *size,
                    mtime_secs,
                    mtime_nanos,
                    staged: false,
                    is_chunked,
                    object_hash: *hash,
                },
            )
        })
        .collect();
    let mut new_index = Index::default();
    for (path, entry) in new_entries {
        new_index.set(path, entry);
    }
    new_index.save(&index_path)?;

    // Finally, update HEAD.
    ws.set_head(&new_head)?;
    Ok(())
}

/// Walk `tree` in parallel, accumulating `path -> (object_hash, size)`
/// for every reachable file. Sibling subtrees are read in parallel via
/// rayon so the recursive object-store fetches overlap on a thread pool
/// instead of stalling end-to-end on disk seeks.
fn flatten_tree_parallel(
    ws: &Workspace,
    tree: &Tree,
    prefix: &str,
) -> Result<BTreeMap<String, (ForgeHash, u64)>> {
    let acc: Mutex<BTreeMap<String, (ForgeHash, u64)>> = Mutex::new(BTreeMap::new());

    tree.entries
        .par_iter()
        .try_for_each(|entry| -> Result<()> {
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{}/{}", prefix, entry.name)
            };
            match entry.kind {
                EntryKind::File | EntryKind::Symlink => {
                    acc.lock().unwrap().insert(path, (entry.hash, entry.size));
                }
                EntryKind::Directory => {
                    let subtree = ws.object_store.get_tree(&entry.hash)?;
                    let sub = flatten_tree_parallel(ws, &subtree, &path)?;
                    let mut guard = acc.lock().unwrap();
                    for (p, v) in sub {
                        guard.insert(p, v);
                    }
                }
            }
            Ok(())
        })?;

    Ok(acc.into_inner().unwrap())
}

/// `git switch <branch>` DWIM: when the local branch doesn't exist, look
/// for a remote-tracking ref (`refs/remotes/<remote>/<branch>`) left by
/// `forge fetch`. If exactly one matches, materialize a local branch at
/// that hash and return its tip. Returns `Ok(None)` when no remote has it,
/// so the caller can produce a clean "branch not found" error.
///
/// Scans every remote-tracking ref on disk (not just `config.remotes`) so
/// a workspace that was hand-populated or whose config was wiped still
/// gets the DWIM. The default remote — when one is configured — is
/// preferred over alphabetical order, matching git's "first hit wins"
/// behavior.
fn try_create_from_remote_ref(ws: &Workspace, name: &str) -> Result<Option<ForgeHash>> {
    let all = ws.list_all_remote_refs()?;
    if all.is_empty() {
        return Ok(None);
    }

    // Find every (remote, hash) pair whose branch matches `name`.
    let matches: Vec<(String, ForgeHash)> = all
        .into_iter()
        .filter_map(|(remote, branch, hash)| (branch == name).then_some((remote, hash)))
        .collect();

    if matches.is_empty() {
        return Ok(None);
    }

    // Prefer the default remote when one is configured, otherwise take
    // the first alphabetical match (list_all_remote_refs returns sorted).
    let preferred_remote = ws
        .config()
        .ok()
        .and_then(|c| c.default_remote().map(|r| r.name.clone()));

    let (remote, tip) = preferred_remote
        .as_ref()
        .and_then(|name| matches.iter().find(|(r, _)| r == name).cloned())
        .unwrap_or_else(|| matches[0].clone());

    ws.set_branch_tip(name, &tip)?;
    println!("Branch '{}' set up to track '{}/{}'", name, remote, name);
    Ok(Some(tip))
}

/// Check if an object in the store is a ChunkedBlob (type byte == 2).
fn is_chunked_object(ws: &Workspace, hash: &ForgeHash) -> bool {
    match ws.object_store.chunks.get(hash) {
        Ok(data) if !data.is_empty() => data[0] == 2,
        _ => false,
    }
}

/// Read a blob's content from the object store.
/// For small files, this is the raw blob data.
/// For chunked files, reassemble from the manifest.
fn read_blob_content(ws: &Workspace, object_hash: &ForgeHash) -> Result<Vec<u8>> {
    let data = ws
        .object_store
        .chunks
        .get(object_hash)
        .map_err(|e| anyhow::anyhow!("Failed to read object {}: {}", object_hash.short(), e))?;

    if data.is_empty() {
        return Ok(data); // Empty file — valid
    }

    if data[0] == 2 {
        // ChunkedBlob manifest — reassemble.
        let manifest: forge_core::object::blob::ChunkedBlob = bincode::deserialize(&data[1..])
            .map_err(|e| anyhow::anyhow!("Failed to deserialize manifest: {}", e))?;
        let content = forge_core::chunk::reassemble_chunks(&manifest, |h| {
            ws.object_store.chunks.get(h).ok()
        })
        .ok_or_else(|| anyhow::anyhow!("Failed to reassemble chunked blob"))?;
        Ok(content)
    } else {
        Ok(data)
    }
}

/// Get mtime of a file as (secs, nanos).
fn mtime_of(path: &std::path::Path) -> (i64, u32) {
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(mtime) = meta.modified() {
            let dur = mtime
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default();
            return (dur.as_secs() as i64, dur.subsec_nanos());
        }
    }
    (0, 0)
}

fn clear_readonly(path: &std::path::Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        if perms.readonly() {
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}
