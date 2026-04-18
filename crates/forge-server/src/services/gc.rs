// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Garbage collection — mark-and-sweep over the per-repo object store.
//!
//! **Mark.** Walk every ref in the repo's `refs` table (branches + tags)
//! as a snapshot root. Snapshots link to their tree and parent snapshots;
//! trees link to child trees and file objects; file objects are either
//! raw blobs (leaves) or [`ChunkedBlob`] manifests whose chunk hashes are
//! leaves. The mark pass records every hash it touches in a `HashSet`.
//!
//! **Sweep.** Walk the live-store shard tree with
//! [`ObjectBackend::iter_all`]; any hash not in the marked set and older
//! than the grace window is deleted. Staging (`_staging/<sid>/…`) is
//! *not* visited by `iter_all`, so in-flight uploads are safe.
//!
//! **Why a grace window.** A CommitPush runs as
//! `promote_into` → ref CAS inside the same DB transaction. Between
//! `promote_into` touching the live tree and the CAS committing, the
//! object is on disk but unreachable from any ref. Without a grace
//! window, GC would race that gap and delete objects the about-to-commit
//! push depends on. 24 h is intentionally generous — disk is cheap,
//! wrong deletions are not.
//!
//! **No session-aware protection.** Session-scoped uploads live under
//! `_staging/` (already skipped) or have just been promoted into live
//! (mtime < grace). The grace window covers both cases, so we don't
//! pay the complexity of cross-checking `session_objects`.
//!
//! **Corrupt or missing links.** If we can't read a reachable object
//! during the mark pass, we log and keep walking. The sweep never
//! deletes a marked hash, so a missing snapshot just leaves its (dead)
//! subtree eligible for sweep — which is the correct outcome.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use bincode;
use forge_core::hash::ForgeHash;
use forge_core::object::blob::ChunkedBlob;
use forge_core::object::snapshot::Snapshot;
use forge_core::object::tree::{EntryKind, Tree};
use forge_core::object::ObjectType;
use forge_core::store::chunk_store::ChunkStore;
use tokio::time::interval;
use tracing::{debug, info, warn};

use crate::storage::backend::MetadataBackend;
use crate::storage::db::MetadataDb;
use crate::storage::fs::FsStorage;

/// Default grace window — objects younger than this are never swept
/// even when unreachable. Picked so a CommitPush that stalls after
/// `promote_into` but before ref-CAS has plenty of time to complete
/// before GC runs.
pub const DEFAULT_GRACE_SECS: i64 = 24 * 60 * 60;

/// Default interval between scheduled sweeps. GC is per-repo and scans
/// the full shard tree; 6 h is a reasonable first approximation for a
/// single-host deploy. Operators with millions of objects should raise
/// it; test clusters lower it.
const SWEEP_INTERVAL_SECS: u64 = 6 * 60 * 60;

/// Per-repo report. The server task logs one of these per cycle; the
/// `forge-server gc` CLI prints them.
#[derive(Debug, Default, Clone)]
pub struct GcReport {
    pub repo: String,
    /// Hashes seen while walking the shard tree (not counting
    /// `_staging/`). Useful sanity check — should equal `marked + swept
    /// + skipped_young`.
    pub scanned: u64,
    /// Hashes reached by the mark pass (any ref's DAG closure).
    pub marked: u64,
    /// Objects actually deleted by the sweep.
    pub swept: u64,
    /// Objects that would have been swept but fell inside the grace
    /// window. Reported so operators can tell "working" from "idle".
    pub skipped_young: u64,
    /// Bytes reclaimed (compressed on-disk size).
    pub bytes_freed: u64,
    /// Non-fatal errors (unreadable object during mark, failed delete,
    /// etc.). The sweep continues past these; a high count should
    /// prompt operator investigation.
    pub errors: u64,
}

/// Scheduled GC task. Mirrors [`crate::services::session_sweeper::spawn`]
/// — one tokio task with a periodic ticker, best-effort work, nothing
/// that can wedge startup.
pub fn spawn(db: Arc<MetadataDb>, fs: Arc<FsStorage>) {
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(SWEEP_INTERVAL_SECS));
        // Skip the immediate first tick — give the server time to
        // finish startup before torching disk I/O at the repo store.
        tick.tick().await;
        loop {
            tick.tick().await;
            match run(&*db, &*fs, DEFAULT_GRACE_SECS, false) {
                Ok(reports) => {
                    let reclaimed: u64 = reports.iter().map(|r| r.swept).sum();
                    let freed: u64 = reports.iter().map(|r| r.bytes_freed).sum();
                    let errors: u64 = reports.iter().map(|r| r.errors).sum();
                    if reclaimed > 0 || errors > 0 {
                        info!(
                            repos = reports.len(),
                            reclaimed,
                            bytes_freed = freed,
                            errors,
                            grace_hours = DEFAULT_GRACE_SECS / 3600,
                            "gc sweep complete"
                        );
                    } else {
                        debug!(repos = reports.len(), "gc sweep: nothing to reclaim");
                    }
                }
                Err(e) => {
                    warn!(error = %e, "gc sweep: top-level failure");
                }
            }
        }
    });
}

/// Run one GC pass over every repo the metadata layer knows about.
/// Used by both the scheduled task and the CLI entry point. `dry_run`
/// skips deletes but still counts what *would* have been swept, so
/// operators can audit before arming the real thing.
pub fn run(
    db: &MetadataDb,
    fs: &FsStorage,
    grace_secs: i64,
    dry_run: bool,
) -> Result<Vec<GcReport>> {
    let repos = db.list_repos().context("list repos for gc")?;
    let mut out = Vec::with_capacity(repos.len());
    for repo in &repos {
        match run_one(db, fs, &repo.name, grace_secs, dry_run) {
            Ok(r) => out.push(r),
            Err(e) => {
                warn!(repo = %repo.name, error = %e, "gc: repo pass failed");
                out.push(GcReport {
                    repo: repo.name.clone(),
                    errors: 1,
                    ..Default::default()
                });
            }
        }
    }
    Ok(out)
}

/// GC a single repo. Public so the CLI can run a single-repo pass
/// (`forge-server gc --repo NAME`).
pub fn run_one(
    db: &MetadataDb,
    fs: &FsStorage,
    repo: &str,
    grace_secs: i64,
    dry_run: bool,
) -> Result<GcReport> {
    let store = fs.repo_store(repo);
    let mut report = GcReport {
        repo: repo.to_string(),
        ..Default::default()
    };

    // Mark: close over every ref.
    let refs = db.get_all_refs(repo).context("get_all_refs")?;
    let marked = mark_reachable(&store, refs.into_iter().map(|(_, h)| h), &mut report.errors);
    report.marked = marked.len() as u64;

    // Sweep: anything in live that isn't marked and is older than grace.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let cutoff = now - grace_secs;

    let iter = match store.iter_all() {
        Ok(it) => it,
        Err(e) => {
            warn!(repo = %repo, error = %e, "gc: iter_all failed");
            report.errors += 1;
            return Ok(report);
        }
    };
    for item in iter {
        let hash = match item {
            Ok(h) => h,
            Err(e) => {
                warn!(repo = %repo, error = %e, "gc: shard walk error");
                report.errors += 1;
                continue;
            }
        };
        report.scanned += 1;

        if marked.contains(&hash) {
            continue;
        }

        // mtime check. ChunkStore uses sharded layout, so reach into
        // the same path resolution via the store. We lean on file_size
        // for existence, and `std::fs::metadata` for the timestamp.
        let path = object_path(store.root(), &hash);
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                // Vanished between iter_all and metadata. Treat as
                // already-swept; don't count as error.
                continue;
            }
        };
        let mtime_secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if mtime_secs >= cutoff {
            report.skipped_young += 1;
            continue;
        }

        let size = meta.len();
        if dry_run {
            report.swept += 1;
            report.bytes_freed += size;
            continue;
        }
        match store.delete(&hash) {
            Ok(true) => {
                report.swept += 1;
                report.bytes_freed += size;
            }
            Ok(false) => {
                // Already gone — someone else deleted it. Not an error.
            }
            Err(e) => {
                warn!(
                    repo = %repo, hash = %hash.to_hex(), error = %e,
                    "gc: delete failed"
                );
                report.errors += 1;
            }
        }
    }

    Ok(report)
}

/// Walk-state kind — tells [`walk_object`] how to interpret the raw
/// bytes. We know snapshot/tree/file by the edge that led us here; we
/// do *not* peek the type byte speculatively because raw blobs and
/// chunks have no type prefix.
#[derive(Copy, Clone, Debug)]
enum Kind {
    Snapshot,
    Tree,
    /// Could be a raw blob *or* a [`ChunkedBlob`] manifest. We detect
    /// the latter by checking byte 0 against `ObjectType::ChunkedBlob`
    /// and only then attempt a bincode deserialise.
    FileContent,
}

/// Mark every object reachable from `roots` (stored as raw hash bytes,
/// from `MetadataDb::get_all_refs`). Returns the closure. Missing or
/// unparseable objects bump the caller's `err_count` and stop the walk
/// at that node — the rest of the DAG continues.
fn mark_reachable<I>(store: &ChunkStore, roots: I, err_count: &mut u64) -> HashSet<ForgeHash>
where
    I: IntoIterator<Item = Vec<u8>>,
{
    let mut seen: HashSet<ForgeHash> = HashSet::new();
    let mut stack: Vec<(ForgeHash, Kind)> = Vec::new();

    for raw in roots {
        match hash_from_bytes(&raw) {
            Some(h) if !h.is_zero() => stack.push((h, Kind::Snapshot)),
            Some(_) => {} // zero-hash sentinel, used as "no ref" — skip.
            None => {
                *err_count += 1;
            }
        }
    }

    while let Some((hash, kind)) = stack.pop() {
        if !seen.insert(hash) {
            continue;
        }
        match walk_object(store, &hash, kind) {
            Ok(children) => stack.extend(children),
            Err(e) => {
                warn!(hash = %hash.to_hex(), ?kind, error = %e, "gc mark: walk failed");
                *err_count += 1;
            }
        }
    }

    seen
}

/// Fetch `hash` and return its outgoing edges. `kind` disambiguates
/// typed (snapshot/tree) from mixed (file content) objects.
fn walk_object(store: &ChunkStore, hash: &ForgeHash, kind: Kind) -> Result<Vec<(ForgeHash, Kind)>> {
    let bytes = store
        .get(hash)
        .with_context(|| format!("read object {} during mark", hash.short()))?;
    match kind {
        Kind::Snapshot => {
            let payload = skip_type_byte(&bytes, ObjectType::Snapshot)?;
            let snap: Snapshot = bincode::deserialize(payload)
                .with_context(|| format!("deserialize snapshot {}", hash.short()))?;
            let mut out = Vec::with_capacity(1 + snap.parents.len());
            out.push((snap.tree, Kind::Tree));
            for p in snap.parents {
                out.push((p, Kind::Snapshot));
            }
            Ok(out)
        }
        Kind::Tree => {
            let payload = skip_type_byte(&bytes, ObjectType::Tree)?;
            let tree: Tree = bincode::deserialize(payload)
                .with_context(|| format!("deserialize tree {}", hash.short()))?;
            Ok(tree
                .entries
                .into_iter()
                .map(|e| {
                    let k = match e.kind {
                        EntryKind::Directory => Kind::Tree,
                        // Symlinks currently store the target as blob
                        // bytes (see object_store.rs); treat them as
                        // file content — the byte-0 check in the next
                        // hop just falls through to leaf.
                        EntryKind::File | EntryKind::Symlink => Kind::FileContent,
                    };
                    (e.hash, k)
                })
                .collect())
        }
        Kind::FileContent => {
            // A `File` entry's hash points at either a raw blob (leaf)
            // or a ChunkedBlob manifest (recurse into chunk hashes).
            // Raw blobs have no type prefix; ChunkedBlob starts with
            // the tag byte. The speculative deserialise on a byte-0
            // collision is cheap — bincode bails on the first wrong
            // field — and the worst case is one false-negative that
            // still leaves the blob reachable as a leaf.
            if bytes.first() == Some(&(ObjectType::ChunkedBlob as u8)) {
                if let Ok(cb) = bincode::deserialize::<ChunkedBlob>(&bytes[1..]) {
                    return Ok(cb
                        .chunks
                        .into_iter()
                        .map(|c| (c.hash, Kind::FileContent))
                        .collect());
                }
            }
            Ok(Vec::new())
        }
    }
}

fn skip_type_byte<'a>(bytes: &'a [u8], expect: ObjectType) -> Result<&'a [u8]> {
    let Some(first) = bytes.first() else {
        anyhow::bail!("object is empty");
    };
    if *first != expect as u8 {
        anyhow::bail!(
            "object type mismatch: expected {:?} ({}), found tag {}",
            expect,
            expect as u8,
            first
        );
    }
    Ok(&bytes[1..])
}

fn hash_from_bytes(raw: &[u8]) -> Option<ForgeHash> {
    if raw.len() != 32 {
        return None;
    }
    ForgeHash::from_hex(&hex::encode(raw)).ok()
}

fn object_path(root: &Path, hash: &ForgeHash) -> std::path::PathBuf {
    let hex = hash.to_hex();
    root.join(&hex[..2]).join(&hex[2..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::object::blob::{ChunkRef, ChunkedBlob};
    use forge_core::object::snapshot::{Author, Snapshot};
    use forge_core::object::tree::{EntryKind, Tree, TreeEntry};
    use forge_core::object::ObjectType;

    fn put_typed<T: serde::Serialize>(
        store: &ChunkStore,
        obj_type: ObjectType,
        obj: &T,
    ) -> ForgeHash {
        let mut buf = vec![obj_type as u8];
        buf.extend_from_slice(&bincode::serialize(obj).unwrap());
        let hash = ForgeHash::from_bytes(&buf);
        store.put(&hash, &buf).unwrap();
        hash
    }

    fn put_raw(store: &ChunkStore, data: &[u8]) -> ForgeHash {
        let hash = ForgeHash::from_bytes(data);
        store.put(&hash, data).unwrap();
        hash
    }

    /// Build a tiny repo: snapshot → tree → (raw blob, chunked manifest → 2 chunks).
    fn seed_repo(store: &ChunkStore) -> (ForgeHash, Vec<ForgeHash>) {
        let raw_blob = put_raw(store, b"hello-raw");

        // Chunks for a chunked blob. We store chunks as raw bytes (no
        // type prefix) — matches `ObjectStore::put_chunk`.
        let c0 = put_raw(store, b"chunk-zero");
        let c1 = put_raw(store, b"chunk-one");
        let manifest = ChunkedBlob {
            total_size: 20,
            chunks: vec![
                ChunkRef {
                    hash: c0,
                    size: 10,
                    offset: 0,
                },
                ChunkRef {
                    hash: c1,
                    size: 10,
                    offset: 10,
                },
            ],
        };
        let chunked_hash = put_typed(store, ObjectType::ChunkedBlob, &manifest);

        let tree = Tree {
            entries: vec![
                TreeEntry {
                    name: "small.txt".into(),
                    kind: EntryKind::File,
                    hash: raw_blob,
                    size: 9,
                },
                TreeEntry {
                    name: "big.bin".into(),
                    kind: EntryKind::File,
                    hash: chunked_hash,
                    size: 20,
                },
            ],
        };
        let tree_hash = put_typed(store, ObjectType::Tree, &tree);

        let snap = Snapshot {
            tree: tree_hash,
            parents: vec![],
            author: Author {
                name: "t".into(),
                email: "t@t".into(),
            },
            message: "seed".into(),
            timestamp: chrono::Utc::now(),
            metadata: Default::default(),
        };
        let snap_hash = put_typed(store, ObjectType::Snapshot, &snap);

        (
            snap_hash,
            vec![raw_blob, c0, c1, chunked_hash, tree_hash, snap_hash],
        )
    }

    #[test]
    fn mark_closes_over_snapshot_tree_manifest_and_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let store = ChunkStore::new(dir.path().join("objects"));
        let (snap_hash, all) = seed_repo(&store);

        let mut errs = 0u64;
        let marked = mark_reachable(
            &store,
            std::iter::once(snap_hash.as_bytes().to_vec()),
            &mut errs,
        );
        assert_eq!(errs, 0);
        for h in &all {
            assert!(marked.contains(h), "missing reachable object {}", h.short());
        }
        assert_eq!(marked.len(), all.len(), "no extras");
    }

    #[test]
    fn sweep_deletes_unreachable_beyond_grace_and_keeps_reachable() {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsStorage::new(dir.path().to_path_buf(), Default::default());
        let store = fs.repo_store("alice/game");
        let (snap_hash, reachable) = seed_repo(&store);

        // Orphan: unreachable, old enough (grace_secs = 0).
        let orphan = put_raw(&store, b"unreachable");

        // We need `get_all_refs` to return our snapshot. Easiest path
        // in a unit test: open a real MetadataDb and register the ref.
        let db_path = dir.path().join("meta.db");
        let db = MetadataDb::open(&db_path).unwrap();
        db.create_repo("alice/game", "test").unwrap();
        db.update_ref(
            "alice/game",
            "refs/heads/main",
            &[0u8; 32],
            snap_hash.as_bytes(),
            false,
        )
        .unwrap();

        // Negative grace forces cutoff > now so every on-disk object
        // (mtime <= now) qualifies as stale. Avoids flakiness on
        // same-second mtime/now equality that a literal `grace=0`
        // would suffer. Dry-run first exercises the accounting path
        // without mutating disk.
        let dry = run_one(&db, &fs, "alice/game", -10, true).unwrap();
        assert_eq!(dry.marked, reachable.len() as u64);
        assert_eq!(dry.swept, 1, "exactly one orphan");
        assert!(store.has(&orphan), "dry run must not delete");

        let real = run_one(&db, &fs, "alice/game", -10, false).unwrap();
        assert_eq!(real.swept, 1);
        assert!(!store.has(&orphan), "orphan swept");
        for h in &reachable {
            assert!(store.has(h), "reachable {} wrongly deleted", h.short());
        }
    }

    #[test]
    fn grace_window_protects_young_orphans() {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsStorage::new(dir.path().to_path_buf(), Default::default());
        let store = fs.repo_store("alice/game");
        let (snap_hash, _) = seed_repo(&store);
        let young_orphan = put_raw(&store, b"brand-new");

        let db_path = dir.path().join("meta.db");
        let db = MetadataDb::open(&db_path).unwrap();
        db.create_repo("alice/game", "test").unwrap();
        db.update_ref(
            "alice/game",
            "refs/heads/main",
            &[0u8; 32],
            snap_hash.as_bytes(),
            false,
        )
        .unwrap();

        // Grace = 1 day — everything just written is still young.
        let report = run_one(&db, &fs, "alice/game", 24 * 60 * 60, false).unwrap();
        assert_eq!(report.swept, 0, "young orphan must survive");
        assert_eq!(report.skipped_young, 1);
        assert!(store.has(&young_orphan));
    }

    #[test]
    fn empty_repo_with_no_refs_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsStorage::new(dir.path().to_path_buf(), Default::default());
        let db_path = dir.path().join("meta.db");
        let db = MetadataDb::open(&db_path).unwrap();
        db.create_repo("ghost", "no pushes").unwrap();

        let report = run_one(&db, &fs, "ghost", 0, false).unwrap();
        assert_eq!(report.scanned, 0);
        assert_eq!(report.marked, 0);
        assert_eq!(report.swept, 0);
        assert_eq!(report.errors, 0);
    }

    #[test]
    fn staging_dir_is_not_scanned_or_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsStorage::new(dir.path().to_path_buf(), Default::default());
        let store = fs.repo_store("alice/game");
        let (snap_hash, reachable) = seed_repo(&store);

        // Drop a bogus object inside `_staging/` — iter_all must skip
        // it, so it never becomes a sweep candidate.
        let staging = fs.session_staging_store("alice/game", "sid-1");
        staging.ensure_shard_dirs().unwrap();
        let phantom = ForgeHash::from_bytes(b"in-staging");
        staging.put(&phantom, b"in-staging").unwrap();

        let db_path = dir.path().join("meta.db");
        let db = MetadataDb::open(&db_path).unwrap();
        db.create_repo("alice/game", "test").unwrap();
        db.update_ref(
            "alice/game",
            "refs/heads/main",
            &[0u8; 32],
            snap_hash.as_bytes(),
            false,
        )
        .unwrap();

        let report = run_one(&db, &fs, "alice/game", -10, false).unwrap();
        assert_eq!(report.swept, 0, "nothing but staging is on disk");
        assert_eq!(report.marked, reachable.len() as u64);

        // Staging file survives.
        let staged_path = fs
            .session_staging_dir("alice/game", "sid-1")
            .join(&phantom.to_hex()[..2])
            .join(&phantom.to_hex()[2..]);
        assert!(staged_path.exists(), "staging must be untouched");
    }

    #[test]
    fn corrupt_snapshot_does_not_poison_the_whole_walk() {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsStorage::new(dir.path().to_path_buf(), Default::default());
        let store = fs.repo_store("alice/game");

        // Reachable snapshot A — perfectly valid.
        let (snap_a, reachable_a) = seed_repo(&store);

        // "Ref B" points at a hash that doesn't exist on disk. The
        // walk must log an error and keep going; it must not abort.
        let missing: ForgeHash = ForgeHash::from_bytes(b"nonexistent-snapshot");

        let db_path = dir.path().join("meta.db");
        let db = MetadataDb::open(&db_path).unwrap();
        db.create_repo("alice/game", "test").unwrap();
        db.update_ref(
            "alice/game",
            "refs/heads/main",
            &[0u8; 32],
            snap_a.as_bytes(),
            false,
        )
        .unwrap();
        db.update_ref(
            "alice/game",
            "refs/heads/broken",
            &[0u8; 32],
            missing.as_bytes(),
            false,
        )
        .unwrap();

        let report = run_one(&db, &fs, "alice/game", -10, false).unwrap();
        // The missing hash is inserted into the seen-set before the
        // walk, so it contributes to `marked` even though it has no
        // descendants. That's harmless for sweep (the hash isn't on
        // disk to begin with) and keeps the mark phase write-once.
        assert!(
            report.marked >= reachable_a.len() as u64,
            "reachable closure must be covered; got marked={}",
            report.marked
        );
        assert!(report.errors >= 1, "must have logged the missing snapshot");
        for h in &reachable_a {
            assert!(store.has(h));
        }
    }
}
