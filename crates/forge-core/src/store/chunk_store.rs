use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::compress;
use crate::error::ForgeError;
use crate::hash::ForgeHash;
use crate::store::backend::ObjectBackend;
use crate::store::pack::PackStore;

/// Subdirectory holding `.pack` + `.idx` pairs underneath a repo's
/// objects root. A packfile is opened read-only at `ChunkStore::new`
/// time and participates in every read-path fallback (has, get,
/// get_raw, iter_all). Writes always land as loose objects — a
/// follow-up `forge-server repack` pass consolidates them.
const PACKS_SUBDIR: &str = "packs";

/// Content-addressable store on disk.
/// Objects are stored in shard directories: `objects/ab/cd1234...`
#[derive(Clone)]
pub struct ChunkStore {
    root: PathBuf,
    /// Read-only pack index. Cloned `ChunkStore`s share the same
    /// backing map via `Arc`. An empty `packs/` directory yields an
    /// empty PackStore — the fall-through paths are all O(1) on
    /// that.
    packs: Arc<PackStore>,
}

impl ChunkStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root: PathBuf = root.into();
        // PackStore::open tolerates a missing dir; still attempt it so
        // a later `repack` that populates the dir is picked up on the
        // next server start without any bookkeeping.
        let packs_dir = root.join(PACKS_SUBDIR);
        let packs = match PackStore::open(&packs_dir) {
            Ok(ps) => Arc::new(ps),
            Err(e) => {
                tracing::warn!(
                    packs_dir = %packs_dir.display(),
                    error = %e,
                    "ChunkStore: failed to open packs dir — continuing without packs"
                );
                Arc::new(PackStore::open(std::path::PathBuf::new()).unwrap_or_else(|_| {
                    // Impossible path: open() tolerates missing dirs. If
                    // it really can't construct, fall back to an empty
                    // store by way of a dummy temp dir.
                    PackStore::open(std::env::temp_dir().join("forge-empty-packs"))
                        .expect("empty PackStore")
                }))
            }
        };
        Self { root, packs }
    }

    /// Directory that [`new`] scans for packs. Exposed so the repack
    /// CLI knows where to drop the freshly-written `.pack` / `.idx`
    /// pair.
    pub fn packs_dir(&self) -> PathBuf {
        self.root.join(PACKS_SUBDIR)
    }

    /// Number of objects currently resolved via packs. Useful for
    /// `forge-server repack --dry-run` reports.
    pub fn packed_object_count(&self) -> usize {
        self.packs.object_count()
    }

    /// Number of open pack files backing this store.
    pub fn pack_file_count(&self) -> usize {
        self.packs.pack_count()
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

    /// Retrieve, decompress, and verify data by hash. Checks the loose
    /// layout first (fast path for freshly written objects), then
    /// falls through to the pack index.
    pub fn get(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        let path = self.object_path(hash);
        if !path.exists() {
            // Pack fall-through. PackStore::get re-verifies BLAKE3
            // just like the loose path, so a tampered pack surfaces
            // here rather than round-tripping as wrong data.
            return self.packs.get(hash);
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
    /// Loose-first, pack fall-through — symmetrical to `get`.
    pub fn get_raw(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        let path = self.object_path(hash);
        if !path.exists() {
            return self.packs.get_raw(hash);
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
        if let Ok(m) = std::fs::metadata(self.object_path(hash)) {
            return Some(m.len());
        }
        self.packs.file_size(hash)
    }

    /// Check if an object exists in the store. Loose layout first,
    /// then pack index — matches the read-path fall-through order.
    pub fn has(&self, hash: &ForgeHash) -> bool {
        self.object_path(hash).exists() || self.packs.has(hash)
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
        let mut hashes: Vec<Result<ForgeHash, ForgeError>> = Vec::new();

        // Pack-resident hashes come first so the enumerator surfaces a
        // full listing even for a repo whose loose tree has been fully
        // repacked (shard dirs emptied). GC's mark-and-sweep relies on
        // seeing every stored hash here. We also record them in a
        // HashSet so the loose walk can skip duplicates — a repack
        // that's written the pack but not yet deleted the loose copy
        // would otherwise surface both.
        let mut packed: std::collections::HashSet<ForgeHash> =
            std::collections::HashSet::with_capacity(self.packs.object_count());
        for h in self.packs.iter_hashes() {
            packed.insert(h);
            hashes.push(Ok(h));
        }

        // Shard dirs that haven't been written to simply don't exist
        // yet. Treat an absent root as an empty loose layer, not an
        // error — happens on a brand-new repo before its first push.
        if !self.root.exists() {
            return Ok(hashes.into_iter());
        }
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
                    Ok(h) => {
                        if packed.contains(&h) {
                            // Pack has this hash already — avoid
                            // double-reporting to iter_all callers.
                            continue;
                        }
                        hashes.push(Ok(h));
                    }
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

    /// Write a packfile under `<root>/packs/` containing the given
    /// `(hash, plaintext)` entries so ChunkStore's fall-through path
    /// has something to resolve against.
    fn seed_pack(root: &std::path::Path, name: &str, entries: &[(ForgeHash, &[u8])]) {
        let compressed: Vec<(ForgeHash, Vec<u8>)> = entries
            .iter()
            .map(|(h, p)| (*h, crate::compress::compress(p).unwrap()))
            .collect();
        crate::store::pack::write_pack(root.join("packs"), name, compressed).unwrap();
    }

    #[test]
    fn has_and_get_fall_through_to_pack_when_loose_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("objects");
        std::fs::create_dir_all(&root).unwrap();

        let payload = b"only-in-pack";
        let hash = ForgeHash::from_bytes(payload);
        seed_pack(&root, "p1", &[(hash, payload)]);

        let store = ChunkStore::new(&root);
        assert!(store.has(&hash));
        assert_eq!(store.get(&hash).unwrap(), payload);
        assert!(store.file_size(&hash).unwrap() > 0);
        assert_eq!(store.packed_object_count(), 1);
        assert_eq!(store.pack_file_count(), 1);
    }

    #[test]
    fn loose_shadows_pack_on_read_but_iter_dedups() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("objects");
        std::fs::create_dir_all(&root).unwrap();

        let payload = b"shared-blob";
        let hash = ForgeHash::from_bytes(payload);

        // Seed the pack first …
        seed_pack(&root, "pfx", &[(hash, payload)]);
        // … then write a loose copy. Both exist on disk.
        let store = ChunkStore::new(&root);
        store.put(&hash, payload).unwrap();

        // Loose wins the read (fast path). Pack is still consulted on
        // other queries so GC sees a consistent world.
        assert!(store.has(&hash));
        assert_eq!(store.get(&hash).unwrap(), payload);

        // iter_all must yield exactly one entry for the duplicated hash.
        let count = store
            .iter_all()
            .unwrap()
            .filter(|r| r.as_ref().unwrap() == &hash)
            .count();
        assert_eq!(count, 1, "duplicated hash across pack + loose must dedup");
    }

    #[test]
    fn iter_all_merges_pack_and_loose_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("objects");
        std::fs::create_dir_all(&root).unwrap();

        let packed_only = ForgeHash::from_bytes(b"p-only");
        let loose_only = ForgeHash::from_bytes(b"l-only");
        seed_pack(&root, "a", &[(packed_only, b"p-only")]);

        let store = ChunkStore::new(&root);
        store.put(&loose_only, b"l-only").unwrap();

        let seen: std::collections::HashSet<_> = store
            .iter_all()
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(seen.contains(&packed_only));
        assert!(seen.contains(&loose_only));
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn missing_packs_dir_is_fine() {
        // Brand-new repo with no `packs/` subdir — ChunkStore still
        // constructs and the pack-aware read path just no-ops.
        let dir = tempfile::tempdir().unwrap();
        let store = ChunkStore::new(dir.path().join("objects"));
        let h = ForgeHash::from_bytes(b"nothing");
        assert!(!store.has(&h));
        assert_eq!(store.pack_file_count(), 0);
        assert_eq!(store.packed_object_count(), 0);
    }
}
