use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::error::ForgeError;
use crate::hash::ForgeHash;

/// The index tracks the state of every tracked file in the working directory.
/// It enables fast `status` by comparing mtime+size before re-hashing.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Index {
    pub version: u32,
    pub entries: BTreeMap<String, IndexEntry>,
}

/// State of a single tracked file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    /// BLAKE3 hash of the file content.
    pub hash: ForgeHash,
    /// File size in bytes.
    pub size: u64,
    /// Last modified time (seconds since epoch).
    pub mtime_secs: i64,
    /// Last modified time (nanoseconds part).
    pub mtime_nanos: u32,
    /// Whether this entry is staged for the next snapshot.
    pub staged: bool,
    /// Whether this file is a chunked blob (large file).
    pub is_chunked: bool,
    /// For chunked files, the hash of the ChunkedBlob manifest.
    /// For small files, same as `hash`.
    pub object_hash: ForgeHash,
}

impl Index {
    /// Load the index from disk.
    pub fn load(path: &Path) -> Result<Self, ForgeError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read(path)?;
        let index = bincode::deserialize(&data)
            .map_err(|e| ForgeError::Serialization(e.to_string()))?;
        Ok(index)
    }

    /// Save the index to disk atomically (write to temp, then rename).
    pub fn save(&self, path: &Path) -> Result<(), ForgeError> {
        let data = bincode::serialize(self)
            .map_err(|e| ForgeError::Serialization(e.to_string()))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Get an entry by workspace-relative path.
    pub fn get(&self, path: &str) -> Option<&IndexEntry> {
        self.entries.get(path)
    }

    /// Insert or update an entry.
    pub fn set(&mut self, path: String, entry: IndexEntry) {
        self.entries.insert(path, entry);
    }

    /// Remove an entry.
    pub fn remove(&mut self, path: &str) -> Option<IndexEntry> {
        self.entries.remove(path)
    }

    /// Get all staged entries.
    pub fn staged_entries(&self) -> Vec<(&String, &IndexEntry)> {
        self.entries
            .iter()
            .filter(|(_, e)| e.staged)
            .collect()
    }

    /// Clear all staged flags.
    pub fn clear_staged(&mut self) {
        for entry in self.entries.values_mut() {
            entry.staged = false;
        }
    }
}
