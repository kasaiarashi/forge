use std::path::{Path, PathBuf};

use crate::compress;
use crate::error::ForgeError;
use crate::hash::ForgeHash;

/// Content-addressable store on disk.
/// Objects are stored in shard directories: `objects/ab/cd1234...`
pub struct ChunkStore {
    root: PathBuf,
}

impl ChunkStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn object_path(&self, hash: &ForgeHash) -> PathBuf {
        let hex = hash.to_hex();
        self.root.join(&hex[..2]).join(&hex[2..])
    }

    /// Store data. Returns true if newly written, false if already existed (dedup).
    pub fn put(&self, hash: &ForgeHash, data: &[u8]) -> Result<bool, ForgeError> {
        let path = self.object_path(hash);
        if path.exists() {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let compressed = compress::compress(data)?;
        // Atomic write: write to temp then rename.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &compressed)?;
        std::fs::rename(&tmp, &path)?;
        Ok(true)
    }

    /// Retrieve, decompress, and verify data by hash.
    pub fn get(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        let path = self.object_path(hash);
        if !path.exists() {
            return Err(ForgeError::ObjectNotFound(hash.to_hex()));
        }
        let compressed = std::fs::read(&path)?;
        let data = compress::decompress(&compressed)?;
        // Verify integrity: recompute hash and compare.
        let actual = ForgeHash::from_bytes(&data);
        if actual != *hash {
            return Err(ForgeError::Other(format!(
                "integrity error: object {} has hash {} on disk",
                hash.to_hex(),
                actual.to_hex()
            )));
        }
        Ok(data)
    }

    /// Read compressed bytes directly from disk (no decompression).
    pub fn get_raw(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        let path = self.object_path(hash);
        if !path.exists() {
            return Err(ForgeError::ObjectNotFound(hash.to_hex()));
        }
        Ok(std::fs::read(&path)?)
    }

    /// Store pre-compressed data directly (no compression).
    pub fn put_raw(&self, hash: &ForgeHash, compressed: &[u8]) -> Result<bool, ForgeError> {
        let path = self.object_path(hash);
        if path.exists() {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, compressed)?;
        std::fs::rename(&tmp, &path)?;
        Ok(true)
    }

    /// Check if an object exists in the store.
    pub fn has(&self, hash: &ForgeHash) -> bool {
        self.object_path(hash).exists()
    }

    /// Delete an object from the store.
    pub fn delete(&self, hash: &ForgeHash) -> Result<bool, ForgeError> {
        let path = self.object_path(hash);
        if path.exists() {
            std::fs::remove_file(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_put_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = ChunkStore::new(dir.path().join("objects"));

        let data = b"hello forge!";
        let hash = ForgeHash::from_bytes(data);

        assert!(!store.has(&hash));
        assert!(store.put(&hash, data).unwrap()); // newly written
        assert!(!store.put(&hash, data).unwrap()); // already exists
        assert!(store.has(&hash));

        let retrieved = store.get(&hash).unwrap();
        assert_eq!(data.as_slice(), retrieved.as_slice());
    }

    #[test]
    fn test_object_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let store = ChunkStore::new(dir.path().join("objects"));
        let hash = ForgeHash::from_bytes(b"nonexistent");
        assert!(store.get(&hash).is_err());
    }
}
