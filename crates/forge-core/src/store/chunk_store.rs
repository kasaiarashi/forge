use std::path::{Path, PathBuf};

use crate::compress;
use crate::error::ForgeError;
use crate::hash::ForgeHash;
use crate::store::backend::ObjectBackend;

/// Content-addressable store on disk.
/// Objects are stored in shard directories: `objects/ab/cd1234...`
#[derive(Clone)]
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

    /// Fast bulk write — skips exists() check, dir creation, and atomic rename.
    ///
    /// Caller must call `ensure_shard_dirs()` first and must have already
    /// verified the object is missing (e.g. via `has_objects`).
    /// Content-addressable stores are self-verifying: a partial write
    /// produces a hash mismatch on read.
    pub fn put_raw_direct(&self, hash: &ForgeHash, compressed: &[u8]) -> Result<(), ForgeError> {
        let path = self.object_path(hash);
        std::fs::write(&path, compressed)?;
        Ok(())
    }

    /// Pre-create all 256 shard directories so per-object writes skip
    /// the `create_dir_all` overhead.
    pub fn ensure_shard_dirs(&self) -> Result<(), ForgeError> {
        for i in 0u8..=255 {
            let dir = self.root.join(format!("{:02x}", i));
            std::fs::create_dir_all(&dir)?;
        }
        Ok(())
    }

    /// Return the on-disk size of a stored object (compressed), or `None`
    /// if the object doesn't exist.  Uses metadata, no file read.
    pub fn file_size(&self, hash: &ForgeHash) -> Option<u64> {
        std::fs::metadata(self.object_path(hash)).ok().map(|m| m.len())
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

    /// Walk every object under the shard tree and yield its
    /// [`ForgeHash`]. Used by GC mark-and-sweep in Phase 3d.
    ///
    /// Returns an iterator so callers can stream millions of hashes
    /// without materialising a Vec. Broken shard entries surface as
    /// `Err` items and don't abort the walk; the GC can decide to
    /// skip or fail.
    pub fn iter_all(
        &self,
    ) -> Result<impl Iterator<Item = Result<ForgeHash, ForgeError>>, ForgeError> {
        // Shard dirs that haven't been written to simply don't exist
        // yet. Treat an absent root as an empty store, not an error —
        // happens on a brand-new repo before its first push.
        if !self.root.exists() {
            let empty: Vec<Result<ForgeHash, ForgeError>> = Vec::new();
            return Ok(empty.into_iter());
        }

        let mut hashes: Vec<Result<ForgeHash, ForgeError>> = Vec::new();
        // Shard directories are 2-hex (00..ff). Staging lives under
        // `_staging/`; skip anything that isn't a 2-hex directory so
        // a stray `_staging` (or tmp files) doesn't show up as an
        // object.
        for shard_entry in std::fs::read_dir(&self.root)? {
            let shard_entry = match shard_entry {
                Ok(e) => e,
                Err(e) => {
                    hashes.push(Err(ForgeError::Io(e)));
                    continue;
                }
            };
            let shard_name = shard_entry.file_name();
            let shard_str = match shard_name.to_str() {
                Some(s) => s,
                None => continue,
            };
            if shard_str.len() != 2 || !shard_str.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            let shard_path = shard_entry.path();
            if !shard_path.is_dir() {
                continue;
            }
            for obj_entry in std::fs::read_dir(&shard_path)? {
                let obj_entry = match obj_entry {
                    Ok(e) => e,
                    Err(e) => {
                        hashes.push(Err(ForgeError::Io(e)));
                        continue;
                    }
                };
                let rest = obj_entry.file_name();
                let rest_str = match rest.to_str() {
                    Some(s) => s,
                    None => continue,
                };
                // ForgeHash is 32 bytes = 64 hex chars. Shard prefix
                // is 2 chars, so the filename rest must be 62 chars.
                // `.tmp` leftovers from crashed put()s are skipped.
                if rest_str.len() != 62 || !rest_str.chars().all(|c| c.is_ascii_hexdigit()) {
                    continue;
                }
                let hex = format!("{shard_str}{rest_str}");
                match ForgeHash::from_hex(&hex) {
                    Ok(h) => hashes.push(Ok(h)),
                    Err(e) => hashes.push(Err(e)),
                }
            }
        }
        Ok(hashes.into_iter())
    }
}

impl ObjectBackend for ChunkStore {
    fn has(&self, hash: &ForgeHash) -> bool {
        ChunkStore::has(self, hash)
    }
    fn get(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        ChunkStore::get(self, hash)
    }
    fn get_raw(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        ChunkStore::get_raw(self, hash)
    }
    fn put(&self, hash: &ForgeHash, data: &[u8]) -> Result<bool, ForgeError> {
        ChunkStore::put(self, hash, data)
    }
    fn put_raw(&self, hash: &ForgeHash, compressed: &[u8]) -> Result<bool, ForgeError> {
        ChunkStore::put_raw(self, hash, compressed)
    }
    fn delete(&self, hash: &ForgeHash) -> Result<bool, ForgeError> {
        ChunkStore::delete(self, hash)
    }
    fn file_size(&self, hash: &ForgeHash) -> Option<u64> {
        ChunkStore::file_size(self, hash)
    }
    fn iter_all<'a>(
        &'a self,
    ) -> Result<Box<dyn Iterator<Item = Result<ForgeHash, ForgeError>> + 'a>, ForgeError> {
        let it = ChunkStore::iter_all(self)?;
        Ok(Box::new(it))
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

    #[test]
    fn iter_all_yields_every_stored_object() {
        let dir = tempfile::tempdir().unwrap();
        let store = ChunkStore::new(dir.path().join("objects"));

        let payloads: Vec<&[u8]> = vec![b"alpha", b"beta", b"gamma"];
        let mut expected = std::collections::HashSet::new();
        for p in &payloads {
            let h = ForgeHash::from_bytes(p);
            store.put(&h, p).unwrap();
            expected.insert(h);
        }

        let mut seen = std::collections::HashSet::new();
        for item in store.iter_all().unwrap() {
            seen.insert(item.unwrap());
        }
        assert_eq!(seen, expected);
    }

    #[test]
    fn iter_all_skips_non_hex_dirs_and_tmp_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("objects");
        let store = ChunkStore::new(&root);

        // Store one real object.
        let payload = b"real-object";
        let h = ForgeHash::from_bytes(payload);
        store.put(&h, payload).unwrap();

        // Drop a staging dir + a tmp file that must not appear in
        // iter_all output — these mimic the server-side staging
        // layout and a crashed put().
        std::fs::create_dir_all(root.join("_staging")).unwrap();
        std::fs::write(root.join("_staging").join("junk"), b"x").unwrap();
        let hex = h.to_hex();
        let shard = root.join(&hex[..2]);
        std::fs::write(shard.join("leftover.tmp"), b"stale").unwrap();

        let items: Vec<_> = store.iter_all().unwrap().collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].as_ref().unwrap(), &h);
    }

    #[test]
    fn iter_all_on_empty_store_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = ChunkStore::new(dir.path().join("objects"));
        // Root doesn't exist yet — must not error.
        let items: Vec<_> = store.iter_all().unwrap().collect();
        assert!(items.is_empty());
    }

    #[test]
    fn object_backend_trait_dispatches_to_inherent_methods() {
        use crate::store::backend::ObjectBackend;

        let dir = tempfile::tempdir().unwrap();
        let store = ChunkStore::new(dir.path().join("objects"));
        let payload = b"trait dispatch check";
        let h = ForgeHash::from_bytes(payload);

        // Go entirely through the trait surface.
        let trait_ref: &dyn ObjectBackend = &store;
        assert!(!trait_ref.has(&h));
        assert!(trait_ref.put(&h, payload).unwrap());
        assert!(trait_ref.has(&h));
        assert_eq!(trait_ref.get(&h).unwrap(), payload);
        assert!(trait_ref.file_size(&h).unwrap() > 0);
        let mut seen = 0;
        for item in trait_ref.iter_all().unwrap() {
            assert_eq!(item.unwrap(), h);
            seen += 1;
        }
        assert_eq!(seen, 1);
        assert!(trait_ref.delete(&h).unwrap());
        assert!(!trait_ref.has(&h));
    }
}
