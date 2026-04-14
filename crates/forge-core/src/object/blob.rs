use serde::{Deserialize, Serialize};

use crate::hash::ForgeHash;

/// A small file stored as a single blob.
/// The blob content is stored separately in the chunk store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blob {
    pub size: u64,
}

/// A large file split into content-defined chunks.
/// Stored as a manifest listing chunk hashes in order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkedBlob {
    /// Total uncompressed file size.
    pub total_size: u64,
    /// Ordered list of chunk references.
    pub chunks: Vec<ChunkRef>,
}

/// A reference to a single chunk within a chunked blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRef {
    /// BLAKE3 hash of the raw (uncompressed) chunk data.
    pub hash: ForgeHash,
    /// Uncompressed size of this chunk.
    pub size: u64,
    /// Byte offset of this chunk within the reassembled file.
    pub offset: u64,
}
