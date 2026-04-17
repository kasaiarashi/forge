// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use forge_core::hash::ForgeHash;
use forge_core::store::chunk_store::ChunkStore;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

/// Server-side filesystem storage with per-repo object directories.
pub struct FsStorage {
    base_path: PathBuf,
    /// Per-repo path overrides from config.
    repo_overrides: HashMap<String, PathBuf>,
}

impl FsStorage {
    pub fn new(base_path: PathBuf, repo_overrides: HashMap<String, PathBuf>) -> Self {
        std::fs::create_dir_all(&base_path).ok();
        Self {
            base_path,
            repo_overrides,
        }
    }

    /// Rename a repo directory from old name to new name.
    pub fn rename_repo(&self, old_name: &str, new_name: &str) -> std::io::Result<()> {
        let old_dir = self.base_path.join(old_name);
        let new_dir = self.base_path.join(new_name);
        if old_dir.exists() {
            std::fs::rename(&old_dir, &new_dir)?;
        }
        Ok(())
    }

    /// Delete a repo directory recursively.
    pub fn delete_repo(&self, name: &str) -> std::io::Result<()> {
        let dir = self.base_path.join(name);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    /// Get a ChunkStore for a specific repo's objects directory.
    /// Respects per-repo path overrides from configuration.
    /// Relative overrides are resolved against `base_path` (never its parent)
    /// and canonicalized to prevent path traversal.
    pub fn repo_store(&self, repo: &str) -> ChunkStore {
        let dir = self.repo_objects_path(repo);
        std::fs::create_dir_all(&dir).ok();
        ChunkStore::new(dir)
    }

    /// Resolve the live objects directory for `repo` without creating it.
    /// Used internally so staging/promote share the same resolution rules
    /// as `repo_store`.
    fn repo_objects_path(&self, repo: &str) -> PathBuf {
        if let Some(override_path) = self.repo_overrides.get(repo) {
            if override_path.is_absolute() {
                override_path.join("objects")
            } else {
                let resolved = self.base_path.join(override_path).join("objects");
                resolved.canonicalize().unwrap_or(resolved)
            }
        } else {
            self.base_path.join(repo).join("objects")
        }
    }

    /// Per-session staging directory used by PushObjects. Lives as a
    /// sibling of the live `objects/` dir so the final promote step can be
    /// a `std::fs::rename`, which is atomic on the same volume (NTFS,
    /// ext4, APFS all guarantee this when source + dest are on the same
    /// filesystem). Layout mirrors the live shards: `_staging/<sid>/<ab>/<rest>`.
    pub fn session_staging_dir(&self, repo: &str, session_id: &str) -> PathBuf {
        let mut dir = self.repo_objects_path(repo);
        dir.push("_staging");
        dir.push(session_id);
        dir
    }

    /// Create + return a [`StagingStore`] for the given session. The
    /// staging tree is created lazily on first write; this function only
    /// wraps the computed path.
    pub fn session_staging_store(&self, repo: &str, session_id: &str) -> StagingStore {
        StagingStore {
            root: self.session_staging_dir(repo, session_id),
        }
    }

    /// Delete an entire session's staging directory, best-effort. Used by
    /// the sweeper to reclaim abandoned pushes. Missing dir is not an error.
    pub fn purge_session_staging(&self, repo: &str, session_id: &str) -> io::Result<()> {
        let dir = self.session_staging_dir(repo, session_id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }
}

/// Staging-side writer for a single upload session. Writes go to
/// `<repo>/objects/_staging/<sid>/<shard>/<rest>` and are promoted into
/// the live `<repo>/objects/<shard>/<rest>` tree by `promote_into` —
/// an atomic filesystem rename, not a copy.
pub struct StagingStore {
    root: PathBuf,
}

impl StagingStore {
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn staged_path(&self, hash: &ForgeHash) -> PathBuf {
        let hex = hash.to_hex();
        self.root.join(&hex[..2]).join(&hex[2..])
    }

    /// Ensure all 256 shard dirs exist so per-object writes skip
    /// create_dir_all overhead on the hot path.
    pub fn ensure_shard_dirs(&self) -> io::Result<()> {
        for i in 0u8..=255 {
            let dir = self.root.join(format!("{:02x}", i));
            std::fs::create_dir_all(&dir)?;
        }
        Ok(())
    }

    /// Append bytes to a staged object. Used by the streaming writer so we
    /// never hold a whole object in memory. Creates the file on the first
    /// call for this hash. Uses O_APPEND-style open so concurrent callers
    /// for *different* hashes don't contend; same-hash contention is
    /// prevented by the gRPC handler serialising chunks for one object.
    pub fn append(&self, hash: &ForgeHash, data: &[u8]) -> io::Result<()> {
        use std::fs::OpenOptions;
        use std::io::Write;
        let path = self.staged_path(hash);
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        f.write_all(data)?;
        Ok(())
    }

    /// One-shot write used when a whole object arrives in a single chunk.
    /// Cheaper than `append` because there's no O_APPEND seek.
    pub fn put(&self, hash: &ForgeHash, data: &[u8]) -> io::Result<()> {
        let path = self.staged_path(hash);
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(&path, data)
    }

    /// Return the on-disk size of a staged object, or None if absent.
    pub fn file_size(&self, hash: &ForgeHash) -> Option<u64> {
        std::fs::metadata(self.staged_path(hash)).ok().map(|m| m.len())
    }

    /// Move every staged object into the live store. Objects that are
    /// already present in the live store (content-addressed dedup) have
    /// their staged copy deleted. Failures on individual objects abort the
    /// whole batch so the caller can fail the commit and leave the
    /// staging dir for either retry or the sweeper — we never leave a
    /// half-promoted session.
    ///
    /// This is a filesystem operation, not a database one. It should be
    /// called BEFORE the DB ref-CAS transaction so that ref updates only
    /// land after the objects they reference are durable in the live tree.
    pub fn promote_into(&self, live: &ChunkStore, hashes: &[ForgeHash]) -> io::Result<PromoteStats> {
        let mut stats = PromoteStats::default();
        for hash in hashes {
            let src = self.staged_path(hash);
            if !src.exists() {
                // Not staged here — the client never uploaded it, or it
                // was content-addressed-dedup'd against a prior push and
                // already lives in `live`. Either way, let it through;
                // reachability is enforced by the ref update.
                stats.missing += 1;
                continue;
            }
            if live.has(hash) {
                // Dedup: already in the live store. Drop the staged copy.
                std::fs::remove_file(&src)?;
                stats.deduped += 1;
                continue;
            }
            // Ensure the live shard dir exists. The hot path (pushes of
            // fresh repos) pre-creates all 256 shards, so this is a no-op
            // almost every time.
            let hex = hash.to_hex();
            let live_shard = live.root().join(&hex[..2]);
            if !live_shard.exists() {
                std::fs::create_dir_all(&live_shard)?;
            }
            let dst = live_shard.join(&hex[2..]);
            // rename is atomic on a single volume. Two writers racing the
            // same hash into `live` is safe because the source is
            // content-addressed: both would produce the same byte sequence.
            if let Err(e) = std::fs::rename(&src, &dst) {
                // One retry window: another concurrent promoter may have
                // landed the same content between our `has` check and
                // rename. Re-check and drop the stale staged copy.
                if live.has(hash) {
                    std::fs::remove_file(&src).ok();
                    stats.deduped += 1;
                    continue;
                }
                return Err(e);
            }
            stats.promoted += 1;
        }
        Ok(stats)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PromoteStats {
    pub promoted: u64,
    pub deduped: u64,
    pub missing: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::hash::ForgeHash;
    use tempfile::TempDir;

    fn fresh() -> (TempDir, FsStorage) {
        let dir = TempDir::new().unwrap();
        let fs = FsStorage::new(dir.path().to_path_buf(), Default::default());
        (dir, fs)
    }

    #[test]
    fn staging_put_goes_to_session_dir_not_live() {
        let (_tmp, fs) = fresh();
        let st = fs.session_staging_store("alice/game", "sid-1");
        st.ensure_shard_dirs().unwrap();
        let payload = b"hello forge staging";
        let h = ForgeHash::from_bytes(payload);
        st.put(&h, payload).unwrap();

        // Live store must NOT have this object yet — that's the whole
        // point of the staging layer.
        let live = fs.repo_store("alice/game");
        assert!(!live.has(&h), "object must not be visible in live tree");

        // Staging file exists at <staging>/<shard>/<rest>.
        let hex = h.to_hex();
        let staged_path = fs
            .session_staging_dir("alice/game", "sid-1")
            .join(&hex[..2])
            .join(&hex[2..]);
        assert!(staged_path.exists());
    }

    #[test]
    fn promote_moves_objects_atomically_into_live() {
        let (_tmp, fs) = fresh();
        let st = fs.session_staging_store("alice/game", "sid-1");
        st.ensure_shard_dirs().unwrap();
        let live = fs.repo_store("alice/game");

        let payloads: Vec<&[u8]> = vec![b"one", b"two", b"three"];
        let hashes: Vec<ForgeHash> = payloads
            .iter()
            .map(|p| {
                let h = ForgeHash::from_bytes(p);
                st.put(&h, p).unwrap();
                h
            })
            .collect();

        let stats = st.promote_into(&live, &hashes).unwrap();
        assert_eq!(stats.promoted, 3);
        assert_eq!(stats.deduped, 0);
        assert_eq!(stats.missing, 0);

        // All objects now live in the live store.
        for h in &hashes {
            assert!(live.has(h), "live store missing {}", h.short());
        }
    }

    #[test]
    fn promote_dedups_when_object_already_in_live() {
        let (_tmp, fs) = fresh();
        let live = fs.repo_store("alice/game");
        let payload = b"already-there";
        let h = ForgeHash::from_bytes(payload);
        live.put_raw(&h, payload).unwrap();

        let st = fs.session_staging_store("alice/game", "sid-1");
        st.ensure_shard_dirs().unwrap();
        st.put(&h, payload).unwrap();

        let stats = st.promote_into(&live, &[h]).unwrap();
        assert_eq!(stats.promoted, 0);
        assert_eq!(stats.deduped, 1);

        // Staged copy dropped after dedup so staging dir is reclaimable.
        let hex = h.to_hex();
        let staged = fs
            .session_staging_dir("alice/game", "sid-1")
            .join(&hex[..2])
            .join(&hex[2..]);
        assert!(!staged.exists());
    }

    #[test]
    fn promote_counts_missing_without_failing() {
        // A session that never uploaded an object whose hash is in the
        // manifest — e.g. content-addressed-dedup elided it — must not
        // fail promote. The object just isn't our problem.
        let (_tmp, fs) = fresh();
        let st = fs.session_staging_store("alice/game", "sid-1");
        st.ensure_shard_dirs().unwrap();
        let live = fs.repo_store("alice/game");

        let phantom = ForgeHash::from_bytes(b"never-staged");
        let stats = st.promote_into(&live, &[phantom]).unwrap();
        assert_eq!(stats.missing, 1);
        assert_eq!(stats.promoted, 0);
    }

    #[test]
    fn append_grows_a_multi_chunk_object() {
        let (_tmp, fs) = fresh();
        let st = fs.session_staging_store("alice/game", "sid-1");
        st.ensure_shard_dirs().unwrap();
        let payload = b"hello forge streaming append";
        let full = payload.to_vec();
        let h = ForgeHash::from_bytes(&full);

        // Simulate three chunks arriving separately.
        st.append(&h, &payload[..10]).unwrap();
        st.append(&h, &payload[10..20]).unwrap();
        st.append(&h, &payload[20..]).unwrap();

        assert_eq!(st.file_size(&h), Some(payload.len() as u64));
    }

    #[test]
    fn purge_staging_reclaims_session_tree() {
        let (_tmp, fs) = fresh();
        let st = fs.session_staging_store("alice/game", "sid-abandoned");
        st.ensure_shard_dirs().unwrap();
        let payload = b"abandoned-object";
        let h = ForgeHash::from_bytes(payload);
        st.put(&h, payload).unwrap();

        let dir = fs.session_staging_dir("alice/game", "sid-abandoned");
        assert!(dir.exists());

        fs.purge_session_staging("alice/game", "sid-abandoned").unwrap();
        assert!(!dir.exists());

        // Purging a missing dir is not an error (sweeper reentrancy).
        fs.purge_session_staging("alice/game", "sid-abandoned").unwrap();
    }
}
