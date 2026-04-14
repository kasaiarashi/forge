use std::collections::BTreeMap;

use crate::hash::ForgeHash;
use crate::object::tree::{EntryKind, Tree};

/// Represents a change between two trees.
#[derive(Debug, Clone)]
pub enum DiffEntry {
    Added {
        path: String,
        hash: ForgeHash,
        size: u64,
    },
    Deleted {
        path: String,
        hash: ForgeHash,
        size: u64,
    },
    Modified {
        path: String,
        old_hash: ForgeHash,
        new_hash: ForgeHash,
        old_size: u64,
        new_size: u64,
    },
}

/// Flatten a tree into a map of path -> (hash, size), recursing into subtrees.
pub fn flatten_tree(
    tree: &Tree,
    prefix: &str,
    get_tree: &impl Fn(&ForgeHash) -> Option<Tree>,
) -> BTreeMap<String, (ForgeHash, u64)> {
    let mut result = BTreeMap::new();
    for entry in &tree.entries {
        let path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };
        match entry.kind {
            EntryKind::File | EntryKind::Symlink => {
                result.insert(path, (entry.hash, entry.size));
            }
            EntryKind::Directory => {
                if let Some(subtree) = get_tree(&entry.hash) {
                    let sub = flatten_tree(&subtree, &path, get_tree);
                    result.extend(sub);
                }
            }
        }
    }
    result
}

/// Diff two flattened file maps, producing a list of changes.
pub fn diff_maps(
    old: &BTreeMap<String, (ForgeHash, u64)>,
    new: &BTreeMap<String, (ForgeHash, u64)>,
) -> Vec<DiffEntry> {
    let mut changes = Vec::new();

    for (path, (new_hash, new_size)) in new {
        match old.get(path) {
            Some((old_hash, old_size)) => {
                if old_hash != new_hash {
                    changes.push(DiffEntry::Modified {
                        path: path.clone(),
                        old_hash: *old_hash,
                        new_hash: *new_hash,
                        old_size: *old_size,
                        new_size: *new_size,
                    });
                }
            }
            None => {
                changes.push(DiffEntry::Added {
                    path: path.clone(),
                    hash: *new_hash,
                    size: *new_size,
                });
            }
        }
    }

    for (path, (hash, size)) in old {
        if !new.contains_key(path) {
            changes.push(DiffEntry::Deleted {
                path: path.clone(),
                hash: *hash,
                size: *size,
            });
        }
    }

    changes.sort_by(|a, b| {
        let path_a = match a {
            DiffEntry::Added { path, .. }
            | DiffEntry::Deleted { path, .. }
            | DiffEntry::Modified { path, .. } => path,
        };
        let path_b = match b {
            DiffEntry::Added { path, .. }
            | DiffEntry::Deleted { path, .. }
            | DiffEntry::Modified { path, .. } => path,
        };
        path_a.cmp(path_b)
    });

    changes
}
