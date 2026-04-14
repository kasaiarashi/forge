use serde::{Deserialize, Serialize};
use std::fmt;

use crate::error::ForgeError;

/// A BLAKE3 hash used as the universal content-address identifier.
/// 32 bytes, displayed as 64 hex characters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ForgeHash([u8; 32]);

impl ForgeHash {
    /// The zero hash, used as a sentinel for "no object".
    pub const ZERO: Self = Self([0u8; 32]);

    /// Hash raw bytes and return the ForgeHash.
    pub fn from_bytes(data: &[u8]) -> Self {
        Self(*blake3::hash(data).as_bytes())
    }

    /// Create a new incremental hasher.
    pub fn hasher() -> blake3::Hasher {
        blake3::Hasher::new()
    }

    /// Get the raw 32-byte hash.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// First byte as 2 hex chars, used as shard directory prefix.
    pub fn shard_prefix(&self) -> String {
        hex::encode(&self.0[..1])
    }

    /// Full 64-character hex string.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Short 12-character hex for display.
    pub fn short(&self) -> String {
        self.to_hex()[..12].to_string()
    }

    /// Parse from a 64-character hex string.
    pub fn from_hex(s: &str) -> Result<Self, ForgeError> {
        let bytes = hex::decode(s)
            .map_err(|e| ForgeError::InvalidHash(e.to_string()))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| ForgeError::InvalidHash("expected 32 bytes".into()))?;
        Ok(Self(arr))
    }

    /// Check if this is the zero hash.
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 32]
    }
}

impl fmt::Display for ForgeHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl fmt::Debug for ForgeHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ForgeHash({})", self.short())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_deterministic() {
        let h1 = ForgeHash::from_bytes(b"hello world");
        let h2 = ForgeHash::from_bytes(b"hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_different_inputs() {
        let h1 = ForgeHash::from_bytes(b"hello");
        let h2 = ForgeHash::from_bytes(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_hex_roundtrip() {
        let h = ForgeHash::from_bytes(b"test data");
        let hex = h.to_hex();
        let h2 = ForgeHash::from_hex(&hex).unwrap();
        assert_eq!(h, h2);
    }

    #[test]
    fn test_zero_hash() {
        assert!(ForgeHash::ZERO.is_zero());
        assert!(!ForgeHash::from_bytes(b"x").is_zero());
    }

    #[test]
    fn test_shard_prefix() {
        let h = ForgeHash::from_bytes(b"test");
        let prefix = h.shard_prefix();
        assert_eq!(prefix.len(), 2);
    }
}
