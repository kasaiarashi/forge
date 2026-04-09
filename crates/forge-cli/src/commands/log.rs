use crate::pager;
use anyhow::Result;
use forge_core::diff::{diff_maps, flatten_tree, DiffEntry};
use forge_core::hash::ForgeHash;
use forge_core::workspace::Workspace;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::fmt::Write as _;

/// A commit with its timestamp, used for chronological ordering in --all mode.
struct TimedCommit {
    hash: ForgeHash,
    timestamp: chrono::DateTime<chrono::Utc>,
}

impl Eq for TimedCommit {}
impl PartialEq for TimedCommit {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}
impl PartialOrd for TimedCommit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TimedCommit {
    fn cmp(&self, other: &Self) -> Ordering {
        self.timestamp.cmp(&other.timestamp)
    }
}

pub fn run(
    count: u32,
    file: Option<String>,
    oneline: bool,
    all: bool,
    no_pager: bool,
    json: bool,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;

    // Build a map of commit hash -> list of branch names pointing to it.
    let head_hash = ws.head_snapshot()?;
    let current_branch = ws.current_branch()?;
    let branches = ws.list_branches()?;

    let mut ref_map: HashMap<ForgeHash, Vec<String>> = HashMap::new();
    for branch in &branches {
        if let Ok(tip) = ws.get_branch_tip(branch) {
            ref_map.entry(tip).or_default().push(branch.clone());
        }
    }

    // Normalize file filter path.
    let filter = file.map(|f| f.replace('\\', "/").trim_start_matches("./").to_string());

    let mut json_entries = Vec::new();
    let mut buffer = String::new();
    let mut shown = 0u32;

    if all {
        // --all: walk commits from all branch tips in chronological order (newest first).
        let mut visited: HashSet<ForgeHash> = HashSet::new();
        let mut heap: BinaryHeap<TimedCommit> = BinaryHeap::new();

        // Seed with all branch tips.
        for branch in &branches {
            if let Ok(tip) = ws.get_branch_tip(branch) {
                if !tip.is_zero() && visited.insert(tip) {
                    if let Ok(snap) = ws.object_store.get_snapshot(&tip) {
                        heap.push(TimedCommit {
                            hash: tip,
                            timestamp: snap.timestamp,
                        });
                    }
                }
            }
        }

        while let Some(tc) = heap.pop() {
            if shown >= count {
                break;
            }

            let snapshot = ws.object_store.get_snapshot(&tc.hash)?;

            // File filter check.
            if let Some(ref filter_path) = filter {
                if !commit_touches_file(&ws, &snapshot, filter_path) {
                    // Enqueue parents and skip.
                    for parent in &snapshot.parents {
                        if !parent.is_zero() && visited.insert(*parent) {
                            if let Ok(ps) = ws.object_store.get_snapshot(parent) {
                                heap.push(TimedCommit {
                                    hash: *parent,
                                    timestamp: ps.timestamp,
                                });
                            }
                        }
                    }
                    continue;
                }
            }

            print_commit(
                &mut buffer,
                &tc.hash,
                &snapshot,
                &ref_map,
                &current_branch,
                &head_hash,
                oneline,
                json,
                &mut json_entries,
            );
            shown += 1;

            // Enqueue parents.
            for parent in &snapshot.parents {
                if !parent.is_zero() && visited.insert(*parent) {
                    if let Ok(ps) = ws.object_store.get_snapshot(parent) {
                        heap.push(TimedCommit {
                            hash: *parent,
                            timestamp: ps.timestamp,
                        });
                    }
                }
            }
        }
    } else {
        // Default: linear walk from HEAD.
        let mut current = head_hash;

        while !current.is_zero() && shown < count {
            let snapshot = ws.object_store.get_snapshot(&current)?;

            if let Some(ref filter_path) = filter {
                if !commit_touches_file(&ws, &snapshot, filter_path) {
                    current = snapshot.parents.first().copied().unwrap_or(ForgeHash::ZERO);
                    continue;
                }
            }

            print_commit(
                &mut buffer,
                &current,
                &snapshot,
                &ref_map,
                &current_branch,
                &head_hash,
                oneline,
                json,
                &mut json_entries,
            );
            shown += 1;

            current = snapshot.parents.first().copied().unwrap_or(ForgeHash::ZERO);
        }
    }

    if json {
        let s = serde_json::to_string_pretty(&json_entries)?;
        buffer.clear();
        buffer.push_str(&s);
        buffer.push('\n');
    } else if shown == 0 {
        buffer.push_str("No commits yet.\n");
    }

    pager::show(buffer, no_pager, json);

    Ok(())
}

fn commit_touches_file(
    ws: &Workspace,
    snapshot: &forge_core::object::snapshot::Snapshot,
    filter_path: &str,
) -> bool {
    let get_tree = |h: &ForgeHash| ws.object_store.get_tree(h).ok();
    let current_flat = ws
        .object_store
        .get_tree(&snapshot.tree)
        .ok()
        .map(|t| flatten_tree(&t, "", &get_tree))
        .unwrap_or_default();

    let parent_flat = snapshot
        .parents
        .first()
        .filter(|h| !h.is_zero())
        .and_then(|h| ws.object_store.get_snapshot(h).ok())
        .and_then(|ps| ws.object_store.get_tree(&ps.tree).ok())
        .map(|t| flatten_tree(&t, "", &get_tree))
        .unwrap_or_default();

    let changes = diff_maps(&parent_flat, &current_flat);
    changes.iter().any(|d| {
        let path = match d {
            DiffEntry::Added { path, .. }
            | DiffEntry::Deleted { path, .. }
            | DiffEntry::Modified { path, .. } => path,
        };
        path == filter_path || path.starts_with(&format!("{}/", filter_path))
    })
}

/// Build the git-style decoration block for a commit, e.g.
/// ` (HEAD -> main, dev)` or ` (HEAD)` for detached HEAD. Returns an empty
/// string if there is nothing to show. Used by both default and oneline modes.
fn format_decorations(
    hash: &ForgeHash,
    ref_map: &HashMap<ForgeHash, Vec<String>>,
    current_branch: &Option<String>,
    head_hash: &ForgeHash,
) -> String {
    if let Some(refs) = ref_map.get(hash) {
        let mut parts: Vec<String> = Vec::new();
        let is_head_commit = *hash == *head_hash;

        for branch in refs {
            // The branch matching the current branch (if HEAD points at this commit)
            // gets the "HEAD -> " prefix and goes first.
            if is_head_commit && current_branch.as_deref() == Some(branch.as_str()) {
                parts.insert(
                    0,
                    format!("\x1b[1;36mHEAD -> \x1b[1;32m{}\x1b[0m", branch),
                );
            } else {
                parts.push(format!("\x1b[1;32m{}\x1b[0m", branch));
            }
        }

        if parts.is_empty() {
            String::new()
        } else {
            format!(
                " \x1b[33m(\x1b[0m{}\x1b[33m)\x1b[0m",
                parts.join("\x1b[33m, \x1b[0m")
            )
        }
    } else if *hash == *head_hash && current_branch.is_none() {
        // Detached HEAD: show "(HEAD)" so the user can see where they are.
        " \x1b[33m(\x1b[1;36mHEAD\x1b[33m)\x1b[0m".to_string()
    } else {
        String::new()
    }
}

fn print_commit(
    out: &mut String,
    hash: &ForgeHash,
    snapshot: &forge_core::object::snapshot::Snapshot,
    ref_map: &HashMap<ForgeHash, Vec<String>>,
    current_branch: &Option<String>,
    head_hash: &ForgeHash,
    oneline: bool,
    json: bool,
    json_entries: &mut Vec<serde_json::Value>,
) {
    if json {
        json_entries.push(serde_json::json!({
            "hash": hash.to_hex(),
            "short_hash": hash.short(),
            "author": {
                "name": snapshot.author.name,
                "email": snapshot.author.email,
            },
            "date": snapshot.timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            "message": snapshot.message,
        }));
        return;
    }

    let decorations = format_decorations(hash, ref_map, current_branch, head_hash);

    if oneline {
        let _ = writeln!(
            out,
            "\x1b[33m{}\x1b[0m{} {}",
            hash.short(),
            decorations,
            snapshot.message
        );
    } else {
        let _ = writeln!(out, "\x1b[33mcommit {}\x1b[0m{}", hash.short(), decorations);
        let _ = writeln!(
            out,
            "Author: {} <{}>",
            snapshot.author.name, snapshot.author.email
        );
        let _ = writeln!(
            out,
            "Date:   {}",
            snapshot.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        );
        let _ = writeln!(out);
        let _ = writeln!(out, "    {}", snapshot.message);
        let _ = writeln!(out);
    }
}
