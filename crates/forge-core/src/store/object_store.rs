use std::path::PathBuf;

use crate::error::ForgeError;
use crate::hash::ForgeHash;
use crate::object::blob::ChunkedBlob;
use crate::object::snapshot::Snapshot;
use crate::object::tree::Tree;
use crate::object::ObjectType;
use crate::store::chunk_store::ChunkStore;

/// Typed wrapper over ChunkStore for serialized Forge objects.
pub struct ObjectStore {
    pub chunks: ChunkStore,
}

impl ObjectStore {
    pub fn new(objects_dir: impl Into<PathBuf>) -> Self {
        Self {
            chunks: ChunkStore::new(objects_dir),
        }
    }

    // -- Snapshot --

    pub fn put_snapshot(&self, snap: &Snapshot) -> Result<ForgeHash, ForgeError> {
        let data = self.serialize(ObjectType::Snapshot, snap)?;
        let hash = ForgeHash::from_bytes(&data);
        self.chunks.put(&hash, &data)?;
        Ok(hash)
    }

    pub fn get_snapshot(&self, hash: &ForgeHash) -> Result<Snapshot, ForgeError> {
        let data = self.chunks.get(hash)?;
        self.deserialize(&data)
    }

    // -- Tree --

    pub fn put_tree(&self, tree: &Tree) -> Result<ForgeHash, ForgeError> {
        let data = self.serialize(ObjectType::Tree, tree)?;
        let hash = ForgeHash::from_bytes(&data);
        self.chunks.put(&hash, &data)?;
        Ok(hash)
    }

    pub fn get_tree(&self, hash: &ForgeHash) -> Result<Tree, ForgeError> {
        let data = self.chunks.get(hash)?;
        self.deserialize(&data)
    }

    // -- ChunkedBlob manifest --

    pub fn put_chunked_blob(&self, blob: &ChunkedBlob) -> Result<ForgeHash, ForgeError> {
        let data = self.serialize(ObjectType::ChunkedBlob, blob)?;
        let hash = ForgeHash::from_bytes(&data);
        self.chunks.put(&hash, &data)?;
        Ok(hash)
    }

    pub fn get_chunked_blob(&self, hash: &ForgeHash) -> Result<ChunkedBlob, ForgeError> {
        let data = self.chunks.get(hash)?;
        self.deserialize(&data)
    }

    // -- Raw blob data (small files stored whole) --

    pub fn put_blob_data(&self, data: &[u8]) -> Result<ForgeHash, ForgeError> {
        let hash = ForgeHash::from_bytes(data);
        self.chunks.put(&hash, data)?;
        Ok(hash)
    }

    pub fn get_blob_data(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        self.chunks.get(hash)
    }

    /// Read file content, automatically reassembling chunked blobs.
    /// Tree entries point to either a raw blob or a ChunkedBlob manifest.
    /// This method handles both cases transparently.
    pub fn read_file(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        let data = self.chunks.get(hash)?;
        // Check if this is a typed object (has a type prefix byte).
        if !data.is_empty() && data[0] == ObjectType::ChunkedBlob as u8 {
            if let Ok(chunked) = self.get_chunked_blob(hash) {
                // Reassemble from chunks.
                return crate::chunk::reassemble_chunks(&chunked, |chunk_hash| {
                    self.chunks.get(chunk_hash).ok()
                })
                .ok_or_else(|| {
                    ForgeError::Other(format!(
                        "failed to reassemble chunked blob {}",
                        hash.short()
                    ))
                });
            }
        }
        // Not a chunked blob — return raw data.
        Ok(data)
    }

    // -- Chunk data (individual chunks of large files) --

    pub fn put_chunk(&self, hash: &ForgeHash, data: &[u8]) -> Result<bool, ForgeError> {
        self.chunks.put(hash, data)
    }

    pub fn get_chunk(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        self.chunks.get(hash)
    }

    pub fn has(&self, hash: &ForgeHash) -> bool {
        self.chunks.has(hash)
    }

    // -- Serialization --

    fn serialize<T: serde::Serialize>(
        &self,
        obj_type: ObjectType,
        obj: &T,
    ) -> Result<Vec<u8>, ForgeError> {
        let mut buf = vec![obj_type as u8];
        let encoded = bincode::serialize(obj)
            .map_err(|e| ForgeError::Serialization(e.to_string()))?;
        buf.extend_from_slice(&encoded);
        Ok(buf)
    }

    fn deserialize<T: serde::de::DeserializeOwned>(&self, data: &[u8]) -> Result<T, ForgeError> {
        if data.len() < 2 {
            return Err(ForgeError::Serialization(
                format!("object too small ({} bytes)", data.len()),
            ));
        }
        // Skip the 1-byte type prefix.
        let obj = bincode::deserialize(&data[1..])
            .map_err(|e| ForgeError::Serialization(e.to_string()))?;
        Ok(obj)
    }
}
