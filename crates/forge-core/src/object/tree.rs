use serde::{Deserialize, Serialize};

use crate::hash::ForgeHash;

/// A directory listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree {
    pub entries: Vec<TreeEntry>,
}

/// A single entry in a tree (file, directory, or symlink).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeEntry {
    /// File or directory name (not a full path).
    pub name: String,
    /// What kind of entry this is.
    pub kind: EntryKind,
    /// Hash of the blob/chunked-blob (for files) or tree (for directories).
    pub hash: ForgeHash,
    /// File size in bytes (0 for directories).
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
}
