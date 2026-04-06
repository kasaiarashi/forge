use crate::hash::ForgeHash;
use crate::object::blob::{ChunkRef, ChunkedBlob};

/// Threshold below which files are stored as a single blob (no chunking).
pub const SMALL_FILE_THRESHOLD: u64 = 1_048_576; // 1 MiB

/// FastCDC parameters tuned for UE assets.
pub const CHUNK_MIN: u32 = 65_536; // 64 KiB
pub const CHUNK_AVG: u32 = 262_144; // 256 KiB
pub const CHUNK_MAX: u32 = 1_048_576; // 1 MiB

/// Result of chunking a file.
pub enum ChunkResult {
    /// File is small enough to store as a single blob.
    WholeFile {
        hash: ForgeHash,
        data: Vec<u8>,
    },
    /// File was split into content-defined chunks.
    Chunked {
        manifest: ChunkedBlob,
        chunks: Vec<(ForgeHash, Vec<u8>)>,
    },
}

/// Chunk a file based on its size. Small files are stored whole,
/// large files are split using FastCDC content-defined chunking.
pub fn chunk_file(data: &[u8]) -> ChunkResult {
    if (data.len() as u64) < SMALL_FILE_THRESHOLD {
        let hash = ForgeHash::from_bytes(data);
        return ChunkResult::WholeFile {
            hash,
            data: data.to_vec(),
        };
    }

    let chunker = fastcdc::v2020::FastCDC::new(data, CHUNK_MIN, CHUNK_AVG, CHUNK_MAX);
    let mut chunks = Vec::new();
    let mut refs = Vec::new();
    let mut offset = 0u64;

    for chunk in chunker {
        let chunk_data = &data[chunk.offset..chunk.offset + chunk.length];
        let hash = ForgeHash::from_bytes(chunk_data);
        refs.push(ChunkRef {
            hash,
            size: chunk.length as u64,
            offset,
        });
        chunks.push((hash, chunk_data.to_vec()));
        offset += chunk.length as u64;
    }

    ChunkResult::Chunked {
        manifest: ChunkedBlob {
            total_size: data.len() as u64,
            chunks: refs,
        },
        chunks,
    }
}

/// Reassemble a chunked blob from its chunks.
pub fn reassemble_chunks(manifest: &ChunkedBlob, get_chunk: impl Fn(&ForgeHash) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let mut result = Vec::with_capacity(manifest.total_size as usize);
    for chunk_ref in &manifest.chunks {
        let data = get_chunk(&chunk_ref.hash)?;
        result.extend_from_slice(&data);
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_small_file_not_chunked() {
        let data = vec![42u8; 100];
        match chunk_file(&data) {
            ChunkResult::WholeFile { hash, data: d } => {
                assert_eq!(d.len(), 100);
                assert_eq!(hash, ForgeHash::from_bytes(&data));
            }
            ChunkResult::Chunked { .. } => panic!("small file should not be chunked"),
        }
    }

    #[test]
    fn test_large_file_chunked() {
        let data = vec![0u8; 2 * 1024 * 1024]; // 2 MiB
        match chunk_file(&data) {
            ChunkResult::WholeFile { .. } => panic!("large file should be chunked"),
            ChunkResult::Chunked { manifest, chunks } => {
                assert!(chunks.len() >= 2);
                assert_eq!(manifest.total_size, data.len() as u64);
            }
        }
    }

    #[test]
    fn test_chunking_deterministic() {
        let data = vec![7u8; 2 * 1024 * 1024];
        let r1 = chunk_file(&data);
        let r2 = chunk_file(&data);
        match (r1, r2) {
            (ChunkResult::Chunked { manifest: m1, .. }, ChunkResult::Chunked { manifest: m2, .. }) => {
                assert_eq!(m1.chunks.len(), m2.chunks.len());
                for (a, b) in m1.chunks.iter().zip(m2.chunks.iter()) {
                    assert_eq!(a.hash, b.hash);
                }
            }
            _ => panic!("both should be chunked"),
        }
    }
}
