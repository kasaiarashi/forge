// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Object-storage backend trait.
//!
//! Live-store surface shared by every concrete implementation. The
//! FS-backed [`crate::store::chunk_store::ChunkStore`] (aliased as
//! [`FsObjectStore`]) is the default; Phase 3b ships an S3 variant
//! that speaks the same interface so the gRPC server can swap at
//! startup without touching its push/pull paths.
//!
//! Staging-side operations (per-session upload directories, promote
//! to live) stay on the server-side `FsStorage` for now — they're
//! not yet exercised by the S3 path. Phase 3b extends this trait
//! with `put_staging`/`promote` once the S3 backend needs them.

use crate::error::ForgeError;
use crate::hash::ForgeHash;

/// Content-addressable object store. Methods are live-store only —
/// staging lives one layer up (see `forge-server::storage::fs`).
///
/// Implementors are `Send + Sync` because the gRPC server clones the
/// backend into every request task; locking stays inside the impl
/// when the underlying store (e.g. filesystem) already serialises.
pub trait ObjectBackend: Send + Sync {
    /// Fast existence probe — cheaper than a read.
    fn has(&self, hash: &ForgeHash) -> bool;

    /// Fetch, decompress, and verify an object. Mismatched content
    /// returns [`ForgeError::Other`] so callers distinguish
    /// "corrupt" from "absent" ([`ForgeError::ObjectNotFound`]).
    fn get(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError>;

    /// Raw compressed bytes, no decompression. Used by the push/pull
    /// hot path to stream bytes straight to the wire without
    /// materialising the decompressed payload server-side.
    fn get_raw(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError>;

    /// Store a plaintext object. Returns `true` on a fresh insert,
    /// `false` when the hash was already present (CAS dedup).
    fn put(&self, hash: &ForgeHash, data: &[u8]) -> Result<bool, ForgeError>;

    /// Store pre-compressed bytes as-is. Callers are responsible for
    /// having compressed with the same codec the store expects —
    /// integrity is not re-verified.
    fn put_raw(&self, hash: &ForgeHash, compressed: &[u8]) -> Result<bool, ForgeError>;

    /// Delete an object. Returns `true` if it existed, `false` if
    /// absent (not an error — reclaim paths rely on this).
    fn delete(&self, hash: &ForgeHash) -> Result<bool, ForgeError>;

    /// Bytes on disk for a stored object (compressed), or `None`
    /// when absent. `metadata`-based — no file read.
    fn file_size(&self, hash: &ForgeHash) -> Option<u64>;

    /// Iterate every object in the store. Yields hashes; used by the
    /// GC mark-and-sweep pass in Phase 3d. Ordering is unspecified.
    ///
    /// Errors are reported per-item so a single broken shard doesn't
    /// abort the whole sweep — the caller decides whether to bail or
    /// log + continue.
    fn iter_all<'a>(
        &'a self,
    ) -> Result<Box<dyn Iterator<Item = Result<ForgeHash, ForgeError>> + 'a>, ForgeError>;
}
