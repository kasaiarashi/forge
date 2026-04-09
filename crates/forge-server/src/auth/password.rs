// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Argon2id wrapper used for both passwords and bearer tokens.
//!
//! Passwords and PATs go through the same hashing function — there is no
//! cryptographic reason to use different algorithms for them, and using one
//! function keeps the call sites simple. The same applies to session tokens.
//!
//! The plaintext is provided by the caller (a user-typed password or a
//! freshly-generated random token from [`crate::auth::tokens`]); we do not
//! generate plaintext in this module.

use anyhow::{anyhow, Result};
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

/// Hash a plaintext value (password or token) with argon2id, producing a
/// PHC-format string suitable for storing in a `TEXT` column.
pub fn hash(plaintext: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(plaintext.as_bytes(), &salt)
        .map_err(|e| anyhow!("argon2 hash failed: {e}"))?
        .to_string();
    Ok(hash)
}

/// Verify a plaintext value against a previously stored argon2id hash.
/// Returns `Ok(true)` on match, `Ok(false)` on mismatch, and `Err` only on
/// malformed/corrupt hash strings.
pub fn verify(plaintext: &str, stored_hash: &str) -> Result<bool> {
    let parsed = PasswordHash::new(stored_hash).map_err(|e| anyhow!("parse hash: {e}"))?;
    Ok(Argon2::default()
        .verify_password(plaintext.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrip() {
        let h = hash("hunter2").unwrap();
        assert!(verify("hunter2", &h).unwrap());
        assert!(!verify("wrong", &h).unwrap());
    }

    #[test]
    fn distinct_hashes_for_same_input() {
        // Salts are random — same plaintext should produce different hashes.
        let a = hash("same").unwrap();
        let b = hash("same").unwrap();
        assert_ne!(a, b);
        assert!(verify("same", &a).unwrap());
        assert!(verify("same", &b).unwrap());
    }

    #[test]
    fn malformed_hash_errors() {
        assert!(verify("anything", "not-a-real-hash").is_err());
    }
}
