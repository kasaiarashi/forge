// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Packfile format for small objects (Phase 3e.1).
//!
//! Storing millions of tiny blobs as loose files kills NTFS and
//! significantly slows remote backup. A packfile is a single
//! concatenation of zstd-compressed blobs plus a side-index mapping
//! each hash to its offset + length inside the pack. Reads become one
//! open + one seek; cold-cache walks no longer enumerate a million
//! directory entries.
//!
//! This module ships the on-disk format + a reader that the object
//! store can fall through to when a loose lookup misses. Writing packs
//! (the "repack" pass) is part of the same module so tests can
//! round-trip without pulling in server code; wiring the pack store
//! into `ChunkStore`'s read path and adding a `forge-server repack`
//! command is Phase 3e.2.
//!
//! ## On-disk layout
//!
//! `<dir>/<name>.pack`:
//! ```text
//!   magic     "FORGEPAK" (8 bytes)
//!   version   u32 LE = 1
//!   count     u32 LE
//!   padding   u32 LE = 0 (reserved for header growth)
//!   entries[count]:
//!     hash    [u8; 32]
//!     length  u32 LE
//!     data    [u8; length]    (zstd-compressed; same codec as loose)
//! ```
//!
//! `<dir>/<name>.idx`:
//! ```text
//!   magic     "FORGEIDX" (8 bytes)
//!   version   u32 LE = 1
//!   count     u32 LE
//!   padding   u32 LE = 0
//!   entries[count]:           (sorted ascending by hash for bsearch)
//!     hash    [u8; 32]
//!     offset  u64 LE          (byte offset into `.pack` of data[0])
//!     length  u32 LE          (length of `data` in the pack)
//! ```
//!
//! Both files are append-only after creation; a repack writes a new
//! pair and atomically renames. Never mutate in place.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::compress;
use crate::error::ForgeError;
use crate::hash::ForgeHash;

const PACK_MAGIC: &[u8; 8] = b"FORGEPAK";
const IDX_MAGIC: &[u8; 8] = b"FORGEIDX";
const FORMAT_VERSION: u32 = 1;

/// Header that precedes entries in both `.pack` and `.idx`. 16 bytes
/// on the wire (magic || version || count || reserved) so a future
/// format tweak has a zero-LOC upgrade path.
const HEADER_BYTES: usize = 8 + 4 + 4 + 4;

/// A packfile reader — index held in memory, data block accessed by
/// seek. Cheap to clone via [`Arc`] because the `Mutex<File>` shares.
///
/// Lookups are O(log n) against the sorted index. Loading the whole
/// index into a HashMap would be O(1) at the cost of ~60 bytes per
/// object; at the scales we care about (>100 k packed objects per
/// pack) that's worth it, so we do build a HashMap alongside.
pub struct Packfile {
    path: PathBuf,
    file: Mutex<File>,
    /// hash → (offset, length) in `<path>.pack`.
    index: HashMap<ForgeHash, (u64, u32)>,
}

impl Packfile {
    /// Open an existing pack pair. `pack_path` is the `.pack` file;
    /// the matching `.idx` must live alongside it with the same stem.
    pub fn open(pack_path: impl AsRef<Path>) -> Result<Self, ForgeError> {
        let pack_path = pack_path.as_ref().to_path_buf();
        let idx_path = pack_path.with_extension("idx");
        let index = read_index_file(&idx_path)?;
        // Validate the `.pack` header up-front so a mismatched magic
        // yields a clear startup error rather than a later seek-read
        // returning garbage.
        let mut file = File::open(&pack_path).map_err(|e| ForgeError::Io(e))?;
        validate_header(&mut file, PACK_MAGIC)?;
        Ok(Self {
            path: pack_path,
            file: Mutex::new(file),
            index,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn has(&self, hash: &ForgeHash) -> bool {
        self.index.contains_key(hash)
    }

    /// Read an object from the pack, decompressed and integrity-checked.
    /// Mirrors [`crate::store::chunk_store::ChunkStore::get`] so callers
    /// can substitute a pack lookup into the same code path.
    pub fn get(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        let compressed = self.get_raw(hash)?;
        let data = compress::decompress(&compressed)?;
        let actual = ForgeHash::from_bytes(&data);
        if actual != *hash {
            return Err(ForgeError::Other(format!(
                "integrity error: pack object {} has hash {} after decompress",
                hash.to_hex(),
                actual.to_hex()
            )));
        }
        Ok(data)
    }

    /// Compressed bytes exactly as they live on disk. No integrity
    /// verification — same semantics as `ChunkStore::get_raw`.
    pub fn get_raw(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        let (offset, length) = *self
            .index
            .get(hash)
            .ok_or_else(|| ForgeError::ObjectNotFound(hash.to_hex()))?;
        let mut file = self.file.lock().expect("pack file mutex poisoned");
        file.seek(SeekFrom::Start(offset)).map_err(ForgeError::Io)?;
        let mut buf = vec![0u8; length as usize];
        file.read_exact(&mut buf).map_err(ForgeError::Io)?;
        Ok(buf)
    }

    /// Size of the compressed blob stored for `hash`, or `None` when
    /// absent. Equivalent of `ChunkStore::file_size` for pack content.
    pub fn file_size(&self, hash: &ForgeHash) -> Option<u64> {
        self.index.get(hash).map(|(_, len)| *len as u64)
    }

    /// Every hash the pack contains. Used by GC to fold pack content
    /// into the iter_all enumeration without a separate walker.
    pub fn hashes(&self) -> impl Iterator<Item = ForgeHash> + '_ {
        self.index.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

/// Build a new `.pack` + `.idx` pair at `<dir>/<name>.{pack,idx}` from
/// the supplied `(hash, compressed_bytes)` iterator. The compressed
/// bytes are written verbatim — callers are responsible for having
/// compressed with the same zstd level the rest of the store uses.
///
/// Writes each file through a `.tmp` sibling + rename so a crash
/// mid-pack never leaves a half-written file that later `Packfile::open`
/// would claim to be valid.
pub fn write_pack(
    dir: impl AsRef<Path>,
    name: &str,
    mut objects: Vec<(ForgeHash, Vec<u8>)>,
) -> Result<WrittenPack, ForgeError> {
    if name.is_empty() {
        return Err(ForgeError::Other("pack name must not be empty".into()));
    }
    // Sort so the `.idx` is binary-searchable without a second pass.
    objects.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).map_err(ForgeError::Io)?;

    let pack_path = dir.join(format!("{name}.pack"));
    let idx_path = dir.join(format!("{name}.idx"));
    let pack_tmp = dir.join(format!("{name}.pack.tmp"));
    let idx_tmp = dir.join(format!("{name}.idx.tmp"));

    // Phase 1: stream pack file, collect index entries in lockstep.
    let count = u32::try_from(objects.len())
        .map_err(|_| ForgeError::Other("pack has more than 2^32 objects".into()))?;
    let mut pack_file = File::create(&pack_tmp).map_err(ForgeError::Io)?;
    write_header(&mut pack_file, PACK_MAGIC, count)?;
    let mut offset = HEADER_BYTES as u64;

    let mut index: Vec<(ForgeHash, u64, u32)> = Vec::with_capacity(objects.len());
    for (hash, compressed) in &objects {
        // Per-entry prelude: hash + length. Lets a "dump pack"
        // debugger walk the file without the `.idx`. Redundant with
        // the `.idx` but cheap and self-describing.
        pack_file
            .write_all(hash.as_bytes())
            .map_err(ForgeError::Io)?;
        let len = u32::try_from(compressed.len())
            .map_err(|_| ForgeError::Other("pack entry > 4 GiB".into()))?;
        pack_file
            .write_all(&len.to_le_bytes())
            .map_err(ForgeError::Io)?;

        let data_offset = offset + 32 + 4;
        pack_file.write_all(compressed).map_err(ForgeError::Io)?;
        index.push((*hash, data_offset, len));

        offset = data_offset + len as u64;
    }
    pack_file.sync_all().map_err(ForgeError::Io)?;
    drop(pack_file);

    // Phase 2: write the `.idx`.
    let mut idx_file = File::create(&idx_tmp).map_err(ForgeError::Io)?;
    write_header(&mut idx_file, IDX_MAGIC, count)?;
    for (hash, data_offset, len) in &index {
        idx_file
            .write_all(hash.as_bytes())
            .map_err(ForgeError::Io)?;
        idx_file
            .write_all(&data_offset.to_le_bytes())
            .map_err(ForgeError::Io)?;
        idx_file
            .write_all(&len.to_le_bytes())
            .map_err(ForgeError::Io)?;
    }
    idx_file.sync_all().map_err(ForgeError::Io)?;
    drop(idx_file);

    // Phase 3: promote both files atomically. Rename the `.idx` last
    // so a crash between the two renames leaves the `.pack` alone —
    // a stale `.pack` without its `.idx` is simply ignored by the
    // scanner.
    std::fs::rename(&pack_tmp, &pack_path).map_err(ForgeError::Io)?;
    std::fs::rename(&idx_tmp, &idx_path).map_err(ForgeError::Io)?;

    Ok(WrittenPack {
        pack_path,
        idx_path,
        count: count as usize,
    })
}

/// Paths + count returned by [`write_pack`] so callers can log or
/// immediately re-open the pack for reads.
#[derive(Debug, Clone)]
pub struct WrittenPack {
    pub pack_path: PathBuf,
    pub idx_path: PathBuf,
    pub count: usize,
}

/// Directory-backed collection of packs. Scans `<dir>/*.idx` on open
/// and keeps every discovered pack loaded for the process lifetime —
/// index memory is small (44 bytes per object), and re-opening a
/// pack is cheap if a future caller wants to hot-swap.
pub struct PackStore {
    dir: PathBuf,
    /// Parallel arrays: each hash maps to the pack index within
    /// [`packs`] that owns it, for O(1) dispatch after the index
    /// lookup.
    by_hash: HashMap<ForgeHash, usize>,
    packs: Vec<Packfile>,
}

impl PackStore {
    /// Open every `<dir>/*.pack` whose `.idx` sibling is also present.
    /// A `.pack` without its `.idx` is skipped — a crashed repack or
    /// an interrupted copy should never be consulted for lookups.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, ForgeError> {
        let dir = dir.as_ref().to_path_buf();
        let mut out = Self {
            dir: dir.clone(),
            by_hash: HashMap::new(),
            packs: Vec::new(),
        };
        if !dir.exists() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(&dir).map_err(ForgeError::Io)? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("pack") {
                continue;
            }
            // Require a matching `.idx` — skip half-written pairs.
            if !path.with_extension("idx").exists() {
                continue;
            }
            match Packfile::open(&path) {
                Ok(pack) => out.register(pack),
                Err(e) => {
                    // Don't blow up the whole store because one pack
                    // is malformed. Log + continue; a GC pass can
                    // resurface the bad pack later.
                    tracing::warn!(
                        pack = %path.display(),
                        error = %e,
                        "pack open failed — skipping"
                    );
                }
            }
        }
        Ok(out)
    }

    fn register(&mut self, pack: Packfile) {
        let idx = self.packs.len();
        for h in pack.hashes() {
            // First pack to register a hash wins. Duplicates across
            // packs are a repack bug; the duplicate in the later
            // pack is effectively shadowed. We log so operators see
            // it, but we don't fail open.
            if self.by_hash.insert(h, idx).is_some() {
                tracing::warn!(
                    hash = %h.to_hex(),
                    pack = %pack.path().display(),
                    "duplicate hash across packs — earlier pack wins"
                );
            }
        }
        self.packs.push(pack);
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn pack_count(&self) -> usize {
        self.packs.len()
    }

    pub fn object_count(&self) -> usize {
        self.by_hash.len()
    }

    pub fn has(&self, hash: &ForgeHash) -> bool {
        self.by_hash.contains_key(hash)
    }

    pub fn get(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        let idx = *self
            .by_hash
            .get(hash)
            .ok_or_else(|| ForgeError::ObjectNotFound(hash.to_hex()))?;
        self.packs[idx].get(hash)
    }

    pub fn get_raw(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        let idx = *self
            .by_hash
            .get(hash)
            .ok_or_else(|| ForgeError::ObjectNotFound(hash.to_hex()))?;
        self.packs[idx].get_raw(hash)
    }

    pub fn file_size(&self, hash: &ForgeHash) -> Option<u64> {
        let idx = *self.by_hash.get(hash)?;
        self.packs[idx].file_size(hash)
    }

    /// Every hash across every pack. Ordering is undefined — same
    /// contract as `ChunkStore::iter_all`.
    pub fn iter_hashes(&self) -> impl Iterator<Item = ForgeHash> + '_ {
        self.by_hash.keys().copied()
    }
}

// ── Header helpers ───────────────────────────────────────────────────────────

fn write_header(w: &mut impl Write, magic: &[u8; 8], count: u32) -> Result<(), ForgeError> {
    w.write_all(magic).map_err(ForgeError::Io)?;
    w.write_all(&FORMAT_VERSION.to_le_bytes())
        .map_err(ForgeError::Io)?;
    w.write_all(&count.to_le_bytes()).map_err(ForgeError::Io)?;
    w.write_all(&0u32.to_le_bytes()).map_err(ForgeError::Io)?;
    Ok(())
}

fn validate_header(r: &mut impl Read, expect_magic: &[u8; 8]) -> Result<u32, ForgeError> {
    let mut magic = [0u8; 8];
    r.read_exact(&mut magic).map_err(ForgeError::Io)?;
    if &magic != expect_magic {
        return Err(ForgeError::Other(format!(
            "bad pack magic: expected {:?}, got {:?}",
            std::str::from_utf8(expect_magic).unwrap_or("?"),
            String::from_utf8_lossy(&magic)
        )));
    }
    let mut version_buf = [0u8; 4];
    r.read_exact(&mut version_buf).map_err(ForgeError::Io)?;
    let version = u32::from_le_bytes(version_buf);
    if version != FORMAT_VERSION {
        return Err(ForgeError::Other(format!(
            "unsupported pack version: {version} (this build speaks {FORMAT_VERSION})"
        )));
    }
    let mut count_buf = [0u8; 4];
    r.read_exact(&mut count_buf).map_err(ForgeError::Io)?;
    let count = u32::from_le_bytes(count_buf);
    // Skip the reserved padding word.
    let mut _pad = [0u8; 4];
    r.read_exact(&mut _pad).map_err(ForgeError::Io)?;
    Ok(count)
}

fn read_index_file(idx_path: &Path) -> Result<HashMap<ForgeHash, (u64, u32)>, ForgeError> {
    let mut file = File::open(idx_path).map_err(ForgeError::Io)?;
    let count = validate_header(&mut file, IDX_MAGIC)?;
    let mut out = HashMap::with_capacity(count as usize);
    let mut entry = [0u8; 32 + 8 + 4];
    for _ in 0..count {
        file.read_exact(&mut entry).map_err(ForgeError::Io)?;
        let hash = ForgeHash::from_hex(&hex::encode(&entry[..32]))
            .map_err(|_| ForgeError::Other("index hash parse failed".into()))?;
        let offset = u64::from_le_bytes(entry[32..40].try_into().unwrap());
        let length = u32::from_le_bytes(entry[40..44].try_into().unwrap());
        // Silently drop duplicates within a single `.idx` — a
        // malformed pack is better than a panicky one.
        out.insert(hash, (offset, length));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compressed(payload: &[u8]) -> (ForgeHash, Vec<u8>) {
        let h = ForgeHash::from_bytes(payload);
        let c = compress::compress(payload).unwrap();
        (h, c)
    }

    #[test]
    fn pack_roundtrip_single_entry() {
        let dir = tempfile::tempdir().unwrap();
        let payload = b"hello-pack";
        let (hash, compressed_bytes) = compressed(payload);
        let written =
            write_pack(dir.path(), "test", vec![(hash, compressed_bytes.clone())]).unwrap();
        assert_eq!(written.count, 1);
        assert!(written.pack_path.exists());
        assert!(written.idx_path.exists());

        let pack = Packfile::open(&written.pack_path).unwrap();
        assert!(pack.has(&hash));
        let got = pack.get(&hash).unwrap();
        assert_eq!(got.as_slice(), payload);
        assert_eq!(
            pack.get_raw(&hash).unwrap().as_slice(),
            compressed_bytes.as_slice()
        );
        assert_eq!(pack.file_size(&hash), Some(compressed_bytes.len() as u64));
    }

    #[test]
    fn pack_roundtrip_many_entries_preserves_every_hash() {
        let dir = tempfile::tempdir().unwrap();
        let payloads: Vec<Vec<u8>> = (0..128u32)
            .map(|i| format!("payload-{i:04}").into_bytes())
            .collect();
        let entries: Vec<(ForgeHash, Vec<u8>)> = payloads.iter().map(|p| compressed(p)).collect();
        let expected_hashes: Vec<ForgeHash> = entries.iter().map(|(h, _)| *h).collect();

        write_pack(dir.path(), "many", entries).unwrap();
        let pack = Packfile::open(dir.path().join("many.pack")).unwrap();
        assert_eq!(pack.len(), 128);

        for (h, p) in expected_hashes.iter().zip(payloads.iter()) {
            assert_eq!(pack.get(h).unwrap().as_slice(), p.as_slice());
        }
    }

    #[test]
    fn pack_get_rejects_absent_hash() {
        let dir = tempfile::tempdir().unwrap();
        let (hash, compressed_bytes) = compressed(b"present");
        write_pack(dir.path(), "t", vec![(hash, compressed_bytes)]).unwrap();
        let pack = Packfile::open(dir.path().join("t.pack")).unwrap();
        let ghost = ForgeHash::from_bytes(b"absent");
        assert!(matches!(
            pack.get(&ghost),
            Err(ForgeError::ObjectNotFound(_))
        ));
        assert_eq!(pack.file_size(&ghost), None);
    }

    #[test]
    fn open_rejects_mismatched_magic() {
        let dir = tempfile::tempdir().unwrap();
        // Craft a minimal bogus `.pack` + `.idx` pair.
        let bad_pack = dir.path().join("bogus.pack");
        let bad_idx = dir.path().join("bogus.idx");
        std::fs::write(&bad_pack, b"NOT-A-PACK123456").unwrap();
        std::fs::write(&bad_idx, b"NOT-AN-IDX123456").unwrap();
        match Packfile::open(&bad_pack) {
            Ok(_) => panic!("expected bad-magic error, got Ok"),
            Err(e) => {
                let msg = format!("{e:?}");
                assert!(msg.contains("magic") || msg.contains("pack"), "{msg}");
            }
        }
    }

    #[test]
    fn pack_integrity_detects_decompressed_hash_mismatch() {
        // Craft a pack whose index claims a hash that doesn't match
        // the decompressed bytes, proving the get() integrity check
        // surfaces the tampering.
        let dir = tempfile::tempdir().unwrap();
        let honest_payload = b"honest-bytes";
        let honest_hash = ForgeHash::from_bytes(honest_payload);
        let honest_compressed = compress::compress(honest_payload).unwrap();
        // Pretend this payload lives under a *different* hash.
        let liar_hash = ForgeHash::from_bytes(b"not-the-real-hash");
        write_pack(
            dir.path(),
            "liar",
            vec![(liar_hash, honest_compressed.clone())],
        )
        .unwrap();
        let pack = Packfile::open(dir.path().join("liar.pack")).unwrap();
        let err = pack.get(&liar_hash).unwrap_err();
        match err {
            ForgeError::Other(msg) => assert!(msg.contains("integrity")),
            other => panic!("expected integrity error, got {other:?}"),
        }
        // Honest hash isn't in the pack, of course — should 404.
        assert!(matches!(
            pack.get(&honest_hash),
            Err(ForgeError::ObjectNotFound(_))
        ));
    }

    #[test]
    fn packstore_scans_dir_and_answers_lookups() {
        let dir = tempfile::tempdir().unwrap();
        let packs_dir = dir.path().join("packs");

        let (h1, c1) = compressed(b"one");
        let (h2, c2) = compressed(b"two");
        let (h3, c3) = compressed(b"three");

        write_pack(&packs_dir, "alpha", vec![(h1, c1), (h2, c2)]).unwrap();
        write_pack(&packs_dir, "beta", vec![(h3, c3)]).unwrap();

        let store = PackStore::open(&packs_dir).unwrap();
        assert_eq!(store.pack_count(), 2);
        assert_eq!(store.object_count(), 3);

        for h in [&h1, &h2, &h3] {
            assert!(store.has(h));
        }
        assert_eq!(store.get(&h1).unwrap(), b"one");
        assert_eq!(store.get(&h2).unwrap(), b"two");
        assert_eq!(store.get(&h3).unwrap(), b"three");

        let hashes: std::collections::HashSet<_> = store.iter_hashes().collect();
        assert!(hashes.contains(&h1));
        assert!(hashes.contains(&h2));
        assert!(hashes.contains(&h3));
    }

    #[test]
    fn packstore_skips_orphan_pack_without_idx() {
        let dir = tempfile::tempdir().unwrap();
        let packs_dir = dir.path().join("packs");
        std::fs::create_dir_all(&packs_dir).unwrap();

        // Real pair.
        let (h, c) = compressed(b"ok");
        write_pack(&packs_dir, "good", vec![(h, c)]).unwrap();

        // Orphan `.pack` — no `.idx`. Must be ignored so a crashed
        // repack can't corrupt lookup results.
        std::fs::write(packs_dir.join("bad.pack"), b"garbage").unwrap();

        let store = PackStore::open(&packs_dir).unwrap();
        assert_eq!(store.pack_count(), 1);
        assert!(store.has(&h));
    }

    #[test]
    fn packstore_open_on_missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = PackStore::open(dir.path().join("does-not-exist")).unwrap();
        assert_eq!(store.pack_count(), 0);
        assert_eq!(store.object_count(), 0);
        assert!(!store.has(&ForgeHash::from_bytes(b"anything")));
    }
}
