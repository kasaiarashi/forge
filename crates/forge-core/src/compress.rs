use std::io::{self, Read};

/// Default zstd compression level (3 is a good speed/ratio tradeoff).
pub const ZSTD_LEVEL: i32 = 3;

/// Maximum decompressed size (2 GiB) to prevent decompression bombs.
const MAX_DECOMPRESSED_SIZE: u64 = 2 * 1024 * 1024 * 1024;

/// Compress data using zstd.
pub fn compress(data: &[u8]) -> io::Result<Vec<u8>> {
    zstd::encode_all(io::Cursor::new(data), ZSTD_LEVEL)
}

/// Decompress zstd-compressed data with a size limit to prevent decompression bombs.
pub fn decompress(data: &[u8]) -> io::Result<Vec<u8>> {
    let decoder = zstd::Decoder::new(io::Cursor::new(data))?;
    let mut limited = decoder.take(MAX_DECOMPRESSED_SIZE + 1);
    let mut result = Vec::new();
    limited.read_to_end(&mut result)?;
    if result.len() as u64 > MAX_DECOMPRESSED_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "decompressed data exceeds maximum allowed size (2 GiB)",
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let original = b"hello world, this is test data for compression";
        let compressed = compress(original).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(original.as_slice(), decompressed.as_slice());
    }

    #[test]
    fn test_compression_reduces_size() {
        // Repetitive data should compress well.
        let data = vec![42u8; 10000];
        let compressed = compress(&data).unwrap();
        assert!(compressed.len() < data.len());
    }

    #[test]
    fn test_empty_data() {
        let compressed = compress(b"").unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert!(decompressed.is_empty());
    }
}
