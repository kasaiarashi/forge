// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Offline repack — consolidates small loose objects into packfiles.
//!
//! Run by the operator via `forge-server repack [--repo NAME]
//! [--max-loose-bytes N]`. Finds every loose object whose compressed
//! on-disk size is at or under the threshold, writes them into a single
//! new pack (UUIDv7-named), then deletes the loose copies. A server
//! restart picks up the new pack automatically — reads fall through
//! transparently.
//!
//! **Offline, not online.** This command does not coordinate with a
//! running server. Run it while the server is stopped, or accept that
//! concurrent pushes may write a loose object whose hash is already in
//! the pack we just wrote (benign — dedup on the loose side costs one
//! stat per duplicated object). The explicit "online repack" is a
//! future phase.
//!
//! **Content preserved.** We read each candidate object's compressed
//! bytes verbatim and copy them into the new pack. No decompress /
//! recompress, so the pack is bit-identical to what the loose layer
//! held — critical for content-addressing to keep working after the
//! move.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use forge_core::hash::ForgeHash;
use forge_core::store::chunk_store::ChunkStore;
use forge_core::store::pack::{write_pack, WrittenPack};
use uuid::Uuid;

use crate::storage::fs::FsStorage;

/// Default threshold for "small enough to pack". 4 KiB matches NTFS's
/// default cluster size — anything under that is paying a full cluster
/// per file, which packfiles collapse to a single allocation.
pub const DEFAULT_MAX_LOOSE_BYTES: u64 = 4 * 1024;

#[derive(Debug, Default, Clone)]
pub struct RepackReport {
    pub repo: String,
    /// Loose objects scanned.
    pub scanned: u64,
    /// Objects included in the new pack.
    pub packed: u64,
    /// Loose copies deleted after pack write.
    pub loose_deleted: u64,
    /// Objects skipped because their on-disk size exceeded the
    /// threshold.
    pub skipped_large: u64,
    /// Objects that the pack already held (from a prior repack). We
    /// leave the pack alone and just delete the loose copy.
    pub already_packed: u64,
    /// Compressed bytes reclaimed from the loose layer.
    pub bytes_loose_before: u64,
    /// Bytes in the newly-written pack (including header / per-entry
    /// hashes). Should be modestly less than `bytes_loose_before`
    /// because we save per-file filesystem overhead.
    pub bytes_pack: u64,
    /// Name of the new pack (UUID, `.pack`/`.idx` stem). Empty when
    /// no pack was written because no candidates matched.
    pub pack_name: String,
    /// Non-fatal errors: unreadable loose file, failed delete, etc.
    /// The report continues past these; a high count should prompt
    /// operator investigation.
    pub errors: u64,
}

/// Run a repack against every repo the server knows about.
pub fn run(
    fs: &FsStorage,
    repos: &[String],
    max_loose_bytes: u64,
    dry_run: bool,
) -> Result<Vec<RepackReport>> {
    let mut out = Vec::with_capacity(repos.len());
    for name in repos {
        match run_one(fs, name, max_loose_bytes, dry_run) {
            Ok(r) => out.push(r),
            Err(e) => {
                tracing::warn!(repo = %name, error = %e, "repack: repo pass failed");
                out.push(RepackReport {
                    repo: name.clone(),
                    errors: 1,
                    ..Default::default()
                });
            }
        }
    }
    Ok(out)
}

/// Repack a single repo. Public so the CLI can target `--repo NAME`.
pub fn run_one(
    fs: &FsStorage,
    repo: &str,
    max_loose_bytes: u64,
    dry_run: bool,
) -> Result<RepackReport> {
    let store = fs.repo_store(repo);
    let mut report = RepackReport {
        repo: repo.to_string(),
        ..Default::default()
    };

    let root = store.root().to_path_buf();
    if !root.exists() {
        // Brand-new repo without a single push — nothing to repack.
        return Ok(report);
    }

    // Pass 1: collect candidates. We read compressed bytes straight
    // out of the loose layer so the pack ends up byte-identical and
    // content-addressing keeps working.
    let mut candidates: Vec<(ForgeHash, Vec<u8>, PathBuf)> = Vec::new();

    for shard_entry in std::fs::read_dir(&root)? {
        let shard_entry = match shard_entry {
            Ok(e) => e,
            Err(_) => {
                report.errors += 1;
                continue;
            }
        };
        let shard_name = shard_entry.file_name();
        let Some(shard_str) = shard_name.to_str() else {
            continue;
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
                Err(_) => {
                    report.errors += 1;
                    continue;
                }
            };
            let rest = obj_entry.file_name();
            let Some(rest_str) = rest.to_str() else {
                continue;
            };
            // Skip `.tmp` leftovers and anything that isn't a clean hash.
            if rest_str.len() != 62 || !rest_str.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            let hex = format!("{shard_str}{rest_str}");
            let Ok(hash) = ForgeHash::from_hex(&hex) else {
                report.errors += 1;
                continue;
            };

            let obj_path = obj_entry.path();
            let meta = match obj_entry.metadata() {
                Ok(m) => m,
                Err(_) => {
                    report.errors += 1;
                    continue;
                }
            };
            let size = meta.len();
            report.scanned += 1;
            report.bytes_loose_before += size;

            if size > max_loose_bytes {
                report.skipped_large += 1;
                continue;
            }

            // Already in a pack from a prior repack? Delete the loose
            // copy and move on — no point re-packing it.
            if store.packed_object_count() > 0 && is_already_packed(&store, &hash) {
                report.already_packed += 1;
                if !dry_run {
                    match std::fs::remove_file(&obj_path) {
                        Ok(_) => report.loose_deleted += 1,
                        Err(e) => {
                            tracing::warn!(
                                repo,
                                hash = %hash.to_hex(),
                                error = %e,
                                "repack: delete loose duplicate failed"
                            );
                            report.errors += 1;
                        }
                    }
                }
                continue;
            }

            let bytes = match std::fs::read(&obj_path) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(
                        repo,
                        hash = %hash.to_hex(),
                        error = %e,
                        "repack: read loose object failed"
                    );
                    report.errors += 1;
                    continue;
                }
            };
            candidates.push((hash, bytes, obj_path));
        }
    }

    if candidates.is_empty() {
        return Ok(report);
    }

    if dry_run {
        // Dry run: report what would have been packed, don't write.
        report.packed = candidates.len() as u64;
        return Ok(report);
    }

    // Pass 2: write the new pack. UUIDv7 is time-ordered so `ls packs/`
    // gives a rough history of repack runs without storing a separate
    // manifest.
    let name = Uuid::now_v7().to_string();
    let packs_dir = store.packs_dir();
    let entries: Vec<(ForgeHash, Vec<u8>)> = candidates
        .iter()
        .map(|(h, c, _)| (*h, c.clone()))
        .collect();
    let WrittenPack {
        pack_path,
        idx_path,
        count,
    } = write_pack(&packs_dir, &name, entries).with_context(|| {
        format!("write pack under {}", packs_dir.display())
    })?;
    report.pack_name = name.clone();
    report.packed = count as u64;
    report.bytes_pack = std::fs::metadata(&pack_path)
        .map(|m| m.len())
        .unwrap_or(0);
    // `.idx` size isn't added to `bytes_pack` — it's ancillary and
    // callers compare compressed-data sizes. Keeping it out avoids
    // misleading the "bytes freed" comparison.
    let _ = idx_path;

    // Pass 3: delete the loose copies. Only after the pack is durable
    // on disk (write_pack fsyncs before rename) so a crash here leaves
    // the pack + the loose files, which is safe — both resolve the
    // same bytes.
    for (_, _, path) in &candidates {
        match std::fs::remove_file(path) {
            Ok(_) => report.loose_deleted += 1,
            Err(e) => {
                tracing::warn!(
                    repo,
                    path = %path.display(),
                    error = %e,
                    "repack: delete loose copy failed — pack is durable, will retry next run"
                );
                report.errors += 1;
            }
        }
    }

    Ok(report)
}

/// Probe the pack index (live within the ChunkStore we opened at
/// entry) for a hash. Used to cheaply skip objects that a prior
/// repack already absorbed. ChunkStore holds the pack state resident
/// after construction; we just ask.
fn is_already_packed(store: &ChunkStore, hash: &ForgeHash) -> bool {
    // `has` would fall through to pack; to distinguish pack-only vs
    // "pack + loose", probe the loose path directly first. Simpler:
    // check pack-count + ask `has` after removing the loose copy
    // from the equation. We just want a quick yes/no on "in any pack".
    // ChunkStore has no direct "pack only" probe; cheapest is this:
    let loose_path = loose_path_for(store.root(), hash);
    if !loose_path.exists() {
        // No loose copy, so `has` answering true means the pack does.
        return store.has(hash);
    }
    // Loose exists too — temporarily ignore it by checking the pack
    // store independently via a fresh iterator. Linear scan is fine
    // here: already_packed is a cold edge case (prior repack left
    // loose orphans).
    store.iter_all().ok().map_or(false, |it| {
        for item in it {
            if let Ok(h) = item {
                if h == *hash {
                    return true;
                }
            }
        }
        false
    })
}

fn loose_path_for(root: &Path, hash: &ForgeHash) -> PathBuf {
    let hex = hash.to_hex();
    root.join(&hex[..2]).join(&hex[2..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::compress;

    fn write_loose(fs: &FsStorage, repo: &str, payload: &[u8]) -> ForgeHash {
        let store = fs.repo_store(repo);
        store.ensure_shard_dirs().unwrap();
        let hash = ForgeHash::from_bytes(payload);
        let compressed = compress::compress(payload).unwrap();
        store.put_raw(&hash, &compressed).unwrap();
        hash
    }

    #[test]
    fn repack_packs_small_and_skips_large() {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsStorage::new(dir.path().to_path_buf(), Default::default());

        // Tiny object → packed.
        let small = write_loose(&fs, "alice/game", b"tiny");
        // Above-threshold object after compression → kept loose. Must
        // be non-trivially compressible or zstd will shrink it under
        // the threshold; fill with a deterministic-but-varied byte
        // stream so the compressed size stays close to the raw size.
        let big_bytes: Vec<u8> = (0..16 * 1024u32)
            .flat_map(|i| i.to_le_bytes())
            .collect();
        let big = write_loose(&fs, "alice/game", &big_bytes);

        let report = run_one(&fs, "alice/game", 1024, false).unwrap();
        assert_eq!(report.packed, 1, "only the tiny object goes into the pack");
        assert_eq!(report.skipped_large, 1);
        assert_eq!(report.loose_deleted, 1);
        assert!(!report.pack_name.is_empty());

        // Reopen the store so it picks up the new pack; lookups
        // continue to work for the packed hash, loose copy still
        // present for the big one.
        let store = fs.repo_store("alice/game");
        assert!(store.has(&small));
        assert!(store.has(&big));
        assert_eq!(store.pack_file_count(), 1);
        assert_eq!(store.packed_object_count(), 1);

        // Small object's loose file is gone.
        let hex = small.to_hex();
        let loose_path = store.root().join(&hex[..2]).join(&hex[2..]);
        assert!(!loose_path.exists(), "loose copy of tiny obj must be gone");
    }

    #[test]
    fn repack_dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsStorage::new(dir.path().to_path_buf(), Default::default());
        let hash = write_loose(&fs, "alice/game", b"xxx");

        let report = run_one(&fs, "alice/game", 1024, true).unwrap();
        assert_eq!(report.packed, 1, "dry run still reports candidates");
        assert_eq!(report.loose_deleted, 0, "dry run must not delete");
        assert_eq!(report.pack_name, "", "dry run writes no pack");

        let store = fs.repo_store("alice/game");
        assert_eq!(store.pack_file_count(), 0);
        let hex = hash.to_hex();
        assert!(store.root().join(&hex[..2]).join(&hex[2..]).exists());
    }

    #[test]
    fn repack_on_empty_repo_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsStorage::new(dir.path().to_path_buf(), Default::default());

        let report = run_one(&fs, "ghost", 1024, false).unwrap();
        assert_eq!(report.scanned, 0);
        assert_eq!(report.packed, 0);
        assert_eq!(report.errors, 0);
    }

    #[test]
    fn rerun_removes_orphan_loose_copies_already_in_pack() {
        let dir = tempfile::tempdir().unwrap();
        let fs = FsStorage::new(dir.path().to_path_buf(), Default::default());
        let hash = write_loose(&fs, "alice/game", b"will-become-duplicate");

        // First pass: writes a pack, deletes loose.
        run_one(&fs, "alice/game", 1024, false).unwrap();

        // Simulate a write that landed a loose copy of the same hash
        // after the pack was created (e.g. concurrent push not
        // coordinated with the repack run).
        let store = fs.repo_store("alice/game");
        let compressed = compress::compress(b"will-become-duplicate").unwrap();
        store.put_raw(&hash, &compressed).unwrap();

        // Second pass should detect "already packed" + clean the
        // loose duplicate. No new pack is written for one duplicate.
        let report = run_one(&fs, "alice/game", 1024, false).unwrap();
        assert_eq!(report.already_packed, 1);
        assert_eq!(report.loose_deleted, 1);
        assert_eq!(report.packed, 0);

        // Loose copy gone; pack still serves reads.
        let hex = hash.to_hex();
        let loose_path = store.root().join(&hex[..2]).join(&hex[2..]);
        assert!(!loose_path.exists());
        let reopened = fs.repo_store("alice/game");
        assert!(reopened.has(&hash));
    }
}
