// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Self-signed CA + leaf certificate bootstrap.
//!
//! Goal: make "turn on TLS" a one-line config change. When
//! `[server.tls].auto_generate = true` and the cert files don't exist yet,
//! we create them on startup using `rcgen`:
//!
//! 1. A self-signed **CA** (`ca.crt` + `ca.key`) valid for 10 years. This is
//!    the trust root operators distribute to clients via `forge trust`.
//! 2. A **leaf** certificate (`server.crt` + `server.key`) signed by that CA,
//!    valid for 1 year, with SANs for every hostname / IP in `san_list`.
//!
//! On subsequent restarts the files are reused as-is — we don't regenerate
//! unless the operator deletes them. This keeps the CA fingerprint stable so
//! clients that have already trusted it don't need to re-pin.
//!
//! Calling `print_fingerprint` from main logs a SHA-256 fingerprint of the
//! CA cert so the operator can read it back over a trusted channel and
//! verify it matches what `forge trust` stored on each client.

use anyhow::{Context, Result};
use rcgen::{
    date_time_ymd, BasicConstraints, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType,
};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Paths the autogen machinery reads and writes.
pub struct TlsPaths {
    pub ca_cert: PathBuf,
    pub ca_key: PathBuf,
    pub leaf_cert: PathBuf,
    pub leaf_key: PathBuf,
}

impl TlsPaths {
    /// Default layout under `<base>/certs/`.
    pub fn under(base: &Path) -> Self {
        let dir = base.join("certs");
        Self {
            ca_cert: dir.join("ca.crt"),
            ca_key: dir.join("ca.key"),
            leaf_cert: dir.join("server.crt"),
            leaf_key: dir.join("server.key"),
        }
    }
}

/// Make sure a CA + leaf exist at the given paths. If both leaf files exist,
/// do nothing and return. Otherwise generate a fresh CA (or reuse an existing
/// one) and mint a new leaf covering `san_list`.
///
/// `san_list` should contain every DNS name or IP the server will be reached
/// at. `"localhost"` and `127.0.0.1` are always added so a loopback smoke
/// test works without extra config.
pub fn ensure(paths: &TlsPaths, san_list: &[String]) -> Result<()> {
    if paths.leaf_cert.exists() && paths.leaf_key.exists() {
        return Ok(());
    }

    if let Some(dir) = paths.leaf_cert.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    // Reuse an existing CA if one is already on disk (e.g. only the leaf
    // was deleted). Otherwise generate a fresh one.
    let (ca_cert_pem, ca_key_pair) = if paths.ca_cert.exists() && paths.ca_key.exists() {
        let key_pem = std::fs::read_to_string(&paths.ca_key)
            .with_context(|| format!("reading {}", paths.ca_key.display()))?;
        let cert_pem = std::fs::read_to_string(&paths.ca_cert)
            .with_context(|| format!("reading {}", paths.ca_cert.display()))?;
        let key = KeyPair::from_pem(&key_pem).context("parse CA key")?;
        (cert_pem, key)
    } else {
        mint_ca(paths)?
    };

    // Re-parse the CA so we can sign the leaf with it.
    let ca_params =
        CertificateParams::from_ca_cert_pem(&ca_cert_pem).context("parse CA cert for signing")?;
    let ca_cert = ca_params
        .self_signed(&ca_key_pair)
        .context("re-sign CA for issuing")?;

    // Build the leaf. SANs include the caller-supplied list plus the
    // always-on loopback entries.
    let mut all_sans: Vec<String> = san_list.to_vec();
    for fallback in ["localhost", "127.0.0.1", "::1"] {
        if !all_sans.iter().any(|s| s == fallback) {
            all_sans.push(fallback.to_string());
        }
    }

    let mut leaf_params = CertificateParams::default();
    leaf_params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Forge VCS Server");
        dn.push(DnType::OrganizationName, "Forge VCS");
        dn.push(DnType::OrganizationalUnitName, "Krishna Teja Mekala");
        dn
    };
    leaf_params.is_ca = IsCa::NoCa;
    leaf_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    leaf_params.subject_alt_names =
        all_sans
            .iter()
            .map(|s| match IpAddr::from_str(s) {
                Ok(ip) => SanType::IpAddress(ip),
                Err(_) => SanType::DnsName(s.as_str().try_into().unwrap_or_else(|_| {
                    "localhost".try_into().expect("localhost is always valid")
                })),
            })
            .collect();

    // Validity window: today through 1 year from today (rounded to whole
    // days). Rotation happens by deleting server.{crt,key} and restarting
    // — the CA stays stable so clients don't need to re-pin.
    let (y, m, d) = today_ymd();
    leaf_params.not_before = date_time_ymd(y, m, d);
    leaf_params.not_after = date_time_ymd(y + 1, m, d);

    let leaf_key = KeyPair::generate().context("generate leaf key")?;
    let leaf_cert = leaf_params
        .signed_by(&leaf_key, &ca_cert, &ca_key_pair)
        .context("sign leaf cert")?;

    // Write the leaf file as a FULL CHAIN: leaf + CA. When tonic's
    // `Identity::from_pem` is handed this, it sends both certificates in
    // the TLS `Certificate` message at handshake time, and `forge trust`
    // (on the client) can capture the CA — which is long-lived and stable
    // — as the trust anchor. If we wrote only the leaf, clients would end
    // up pinning a cert that rotates every year, forcing them to re-trust.
    let mut chain_pem = leaf_cert.pem();
    if !chain_pem.ends_with('\n') {
        chain_pem.push('\n');
    }
    chain_pem.push_str(&ca_cert_pem);

    std::fs::write(&paths.leaf_cert, chain_pem)
        .with_context(|| format!("writing {}", paths.leaf_cert.display()))?;
    std::fs::write(&paths.leaf_key, leaf_key.serialize_pem())
        .with_context(|| format!("writing {}", paths.leaf_key.display()))?;

    Ok(())
}

fn mint_ca(paths: &TlsPaths) -> Result<(String, KeyPair)> {
    let mut params = CertificateParams::default();
    params.distinguished_name = {
        // Rich DN — browsers, certutil, and `forge trust` all display
        // these fields, so the user sees "Forge VCS" instead of an
        // anonymous "forge-server local CA" when inspecting the cert.
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Forge VCS Local CA");
        dn.push(DnType::OrganizationName, "Forge VCS");
        dn.push(DnType::OrganizationalUnitName, "Krishna Teja Mekala");
        dn
    };
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let (y, m, d) = today_ymd();
    params.not_before = date_time_ymd(y, m, d);
    params.not_after = date_time_ymd(y + 10, m, d); // 10 years

    let key = KeyPair::generate().context("generate CA key")?;
    let ca = params.self_signed(&key).context("self-sign CA")?;
    let cert_pem = ca.pem();
    std::fs::write(&paths.ca_cert, &cert_pem)
        .with_context(|| format!("writing {}", paths.ca_cert.display()))?;
    std::fs::write(&paths.ca_key, key.serialize_pem())
        .with_context(|| format!("writing {}", paths.ca_key.display()))?;
    Ok((cert_pem, key))
}

/// Compute the SHA-256 fingerprint of a PEM-encoded cert file, formatted as
/// colon-separated hex bytes — the same format `openssl x509 -fingerprint`
/// prints. Returns `None` on read/parse errors rather than failing the boot.
pub fn cert_fingerprint(path: &Path) -> Option<String> {
    let pem = std::fs::read_to_string(path).ok()?;
    // Strip PEM framing, base64-decode, then hash the DER bytes. We roll
    // this by hand rather than pulling in a full x509 parser — rustls has
    // its own but it's private API and we only need the hash.
    let der = pem_to_der(&pem)?;
    let mut hasher = Sha256::new();
    hasher.update(&der);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 3);
    for (i, b) in digest.iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        out.push_str(&format!("{b:02X}"));
    }
    Some(out)
}

/// Current UTC date as (year, month-of-year 1-12, day-of-month 1-31).
/// Uses chrono because the rest of forge already depends on it, avoiding
/// a new `time`-crate dep just for this helper.
fn today_ymd() -> (i32, u8, u8) {
    use chrono::{Datelike, Utc};
    let now = Utc::now().date_naive();
    (now.year(), now.month() as u8, now.day() as u8)
}

fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    let mut in_cert = false;
    let mut base64 = String::new();
    for line in pem.lines() {
        let line = line.trim();
        if line.starts_with("-----BEGIN CERTIFICATE-----") {
            in_cert = true;
            continue;
        }
        if line.starts_with("-----END CERTIFICATE-----") {
            break;
        }
        if in_cert {
            base64.push_str(line);
        }
    }
    if base64.is_empty() {
        return None;
    }
    decode_base64(&base64)
}

/// Tiny standalone base64 decoder — avoids pulling the `base64` crate just
/// to hash one cert during startup. RFC 4648 alphabet, padding optional.
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let clean: Vec<u8> = s
        .bytes()
        .filter(|b| *b != b'=' && !b.is_ascii_whitespace())
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for c in clean {
        let v = val(c)? as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1u32 << bits).saturating_sub(1);
        }
    }
    Some(out)
}
