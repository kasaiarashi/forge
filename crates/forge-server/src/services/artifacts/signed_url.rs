// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Short-lived signed artifact URLs. Used by the web UI so a browser can
//! download an artifact through an HTTP handler without re-doing gRPC auth.
//!
//! Format: `artifact_id.exp.sig`, where `sig = HMAC-SHA256(master_key,
//! "<artifact_id>.<exp>")`. `exp` is a unix timestamp in seconds; the
//! verifier rejects anything in the past. TTL defaults to 15 minutes — the
//! URLs only have to survive the time between a click and the browser
//! finishing the range request.

use anyhow::{bail, Result};
use sha2::Sha256;
use hmac::{Hmac, Mac};

type HmacSha256 = Hmac<Sha256>;

pub const DEFAULT_TTL_SECS: i64 = 15 * 60;

pub fn sign(master_key: &[u8], artifact_id: i64, ttl_secs: i64) -> String {
    let exp = chrono::Utc::now().timestamp() + ttl_secs;
    let payload = format!("{}.{}", artifact_id, exp);
    let mut mac = HmacSha256::new_from_slice(master_key).expect("hmac key len");
    mac.update(payload.as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());
    format!("{payload}.{sig}")
}

pub fn verify(master_key: &[u8], token: &str) -> Result<i64> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        bail!("malformed signed url token");
    }
    let artifact_id: i64 = parts[0].parse().map_err(|_| anyhow::anyhow!("bad id"))?;
    let exp: i64 = parts[1].parse().map_err(|_| anyhow::anyhow!("bad exp"))?;
    if chrono::Utc::now().timestamp() > exp {
        bail!("signed url expired");
    }
    let payload = format!("{}.{}", artifact_id, exp);
    let mut mac = HmacSha256::new_from_slice(master_key).expect("hmac key len");
    mac.update(payload.as_bytes());
    let expected = mac.finalize().into_bytes();
    let got = hex::decode(parts[2]).map_err(|_| anyhow::anyhow!("bad sig hex"))?;
    if got.as_slice() != expected.as_slice() {
        bail!("signed url signature mismatch");
    }
    Ok(artifact_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let key = [7u8; 32];
        let tok = sign(&key, 42, 60);
        let id = verify(&key, &tok).unwrap();
        assert_eq!(id, 42);
    }

    #[test]
    fn tampered_signature_rejected() {
        let key = [7u8; 32];
        let mut tok = sign(&key, 42, 60);
        // Flip the last hex char.
        let n = tok.len();
        let ch = if tok.as_bytes()[n - 1] == b'0' { '1' } else { '0' };
        tok.replace_range(n - 1..n, &ch.to_string());
        assert!(verify(&key, &tok).is_err());
    }

    #[test]
    fn expired_rejected() {
        let key = [7u8; 32];
        let tok = sign(&key, 42, -10);
        assert!(verify(&key, &tok).is_err());
    }
}
