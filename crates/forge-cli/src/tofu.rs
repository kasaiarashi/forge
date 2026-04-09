// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Trust-on-first-use infrastructure shared between `forge login` and
//! `forge trust`.
//!
//! The first time the CLI talks to a self-signed forge server, one of two
//! things happens:
//!
//! 1. **The cert is already trusted** — either because the operator runs a
//!    real public CA (Let's Encrypt, corporate PKI) or because this user
//!    previously pinned it. [`ensure_trusted`] returns silently and the
//!    caller proceeds with the normal gRPC connect.
//!
//! 2. **The cert isn't trusted** — [`ensure_trusted`] opens a lenient TLS
//!    handshake to capture the chain, shows the user the SHA-256
//!    fingerprint, prompts for confirmation (or honors `auto_yes` in
//!    scripts), and writes the pin to `~/.forge/trusted/<host>_<port>.pem`.
//!    The caller can then retry the normal strict connect path and it'll
//!    succeed via the freshly-installed pin.
//!
//! This matches the SSH host-key flow: one manual verification the first
//! time, silent success forever after.

use anyhow::{anyhow, bail, Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

// ── Public API ───────────────────────────────────────────────────────────────

/// Make sure the CLI can reach `server_url` via a trusted TLS path. Returns
/// `Ok(())` when:
///
/// - `server_url` is already pinned in `~/.forge/trusted/`
/// - `server_url`'s cert validates against the public WebPKI trust store
/// - the operator (interactive) confirms the presented fingerprint, OR
///   `auto_yes` is true (scripts / `forge login --yes`)
///
/// Returns an error when the server is unreachable, the presented chain is
/// empty, or the user declines the prompt.
pub async fn ensure_trusted(server_url: &str, auto_yes: bool) -> Result<()> {
    if !server_url.starts_with("https://") {
        return Ok(());
    }
    if has_pin(server_url) {
        return Ok(());
    }

    // Check if the cert is already trusted by the system store. If so,
    // don't pin — pinning a publicly-trusted cert would lock the CLI to a
    // specific leaf that Let's Encrypt rotates every 60-90 days.
    if strict_handshake_ok(server_url).await {
        return Ok(());
    }

    // Untrusted cert — TOFU flow.
    let parsed = parse_url(server_url).ok_or_else(|| anyhow!("unparseable URL"))?;
    let chain = fetch_peer_chain(&parsed.host, parsed.port)
        .await
        .with_context(|| {
            format!(
                "could not reach {server_url} to inspect its TLS certificate"
            )
        })?;
    if chain.is_empty() {
        bail!("server at {server_url} presented no certificates");
    }

    // The CA is the last entry in the chain (forge-server sends leaf + CA);
    // the leaf is the first. If the server only sent a leaf we pin the leaf,
    // which means the user has to re-trust on leaf rotation. forge-server's
    // auto-gen always sends both, so the CA path is the common case.
    let leaf = chain.first().unwrap();
    let anchor = chain.last().unwrap();

    print_fingerprint_prompt(server_url, leaf, anchor);

    if auto_yes {
        eprintln!("Auto-trusting (non-interactive or --yes).");
    } else if !io::stdin().is_terminal() {
        bail!(
            "cannot prompt for trust: stdin is not a terminal. Re-run with \
             --yes to accept the fingerprint automatically, or run \
             `forge trust {server_url}` first."
        );
    } else {
        if !prompt_yes_no("Trust this certificate and continue? [y/N] ")? {
            bail!("trust declined; aborting");
        }
    }

    write_pin(server_url, anchor)?;
    println!("Pinned to {}.", pin_path(server_url)?.display());
    println!();
    Ok(())
}

/// Compute the SHA-256 fingerprint of a DER-encoded certificate, formatted
/// as colon-separated uppercase hex bytes (the same shape `openssl x509
/// -fingerprint -sha256` prints).
pub fn sha256_fingerprint(der: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(der);
    let digest = h.finalize();
    let mut out = String::with_capacity(digest.len() * 3);
    for (i, b) in digest.iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        out.push_str(&format!("{b:02X}"));
    }
    out
}

// ── Internals ────────────────────────────────────────────────────────────────

fn print_fingerprint_prompt(url: &str, leaf: &[u8], anchor: &[u8]) {
    let leaf_fp = sha256_fingerprint(leaf);
    let anchor_fp = sha256_fingerprint(anchor);
    println!();
    println!(
        "This is the first time connecting to {url} and the server's"
    );
    println!("certificate isn't in your system trust store.");
    println!();
    println!("  Leaf SHA-256:   {leaf_fp}");
    if leaf_fp != anchor_fp {
        println!("  Anchor SHA-256: {anchor_fp}");
    }
    println!();
    println!(
        "Compare this against the fingerprint the server operator printed"
    );
    println!("on startup (look for 'TLS CA fingerprint' in the forge-server logs)");
    println!("before accepting.");
    println!();
}

fn prompt_yes_no(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn has_pin(server_url: &str) -> bool {
    pin_path(server_url).map(|p| p.exists()).unwrap_or(false)
}

fn pin_path(server_url: &str) -> Result<PathBuf> {
    let parsed =
        parse_url(server_url).ok_or_else(|| anyhow!("unparseable URL: {server_url}"))?;
    let sanitized: String = parsed
        .host
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => c,
            _ => '_',
        })
        .collect();
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?;
    Ok(home
        .join(".forge")
        .join("trusted")
        .join(format!("{sanitized}_{}.pem", parsed.port)))
}

fn write_pin(server_url: &str, der: &[u8]) -> Result<()> {
    let path = pin_path(server_url)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let pem = der_to_pem(der);
    std::fs::write(&path, pem).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Try a strict TLS handshake using the public WebPKI roots. Returns `true`
/// when the server's cert chain validates against the system trust store,
/// `false` otherwise. Any errors (connection refused, timeout, bad cert)
/// become `false`; the caller handles retry via the lenient TOFU path.
async fn strict_handshake_ok(server_url: &str) -> bool {
    let parsed = match parse_url(server_url) {
        Some(p) => p,
        None => return false,
    };
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(
        webpki_roots::TLS_SERVER_ROOTS
            .iter()
            .cloned(),
    );
    let cfg = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(cfg));

    let stream = match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        TcpStream::connect((parsed.host.as_str(), parsed.port)),
    )
    .await
    {
        Ok(Ok(s)) => s,
        _ => return false,
    };
    let sname = match ServerName::try_from(parsed.host.clone()) {
        Ok(n) => n,
        Err(_) => return false,
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        connector.connect(sname, stream),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

// ── TLS + cert helpers (lenient verifier, raw TCP + handshake) ──────────────

/// Open a TLS connection to `host:port` with a verifier that accepts any
/// certificate, then return the full peer certificate chain in DER bytes.
/// Used exclusively for TOFU capture; never used on the hot gRPC path.
pub async fn fetch_peer_chain(host: &str, port: u16) -> Result<Vec<Vec<u8>>> {
    let cfg = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(CaptureVerifier))
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(cfg));

    let stream = TcpStream::connect((host, port))
        .await
        .with_context(|| format!("TCP connect to {host}:{port}"))?;
    let sname = ServerName::try_from(host.to_string())
        .map_err(|e| anyhow!("invalid server name '{host}': {e}"))?;
    let mut tls = connector
        .connect(sname, stream)
        .await
        .with_context(|| format!("TLS handshake with {host}:{port}"))?;

    let chain = {
        let (_, conn) = tls.get_ref();
        conn.peer_certificates()
            .ok_or_else(|| anyhow!("server did not present any certificates"))?
            .iter()
            .map(|c| c.as_ref().to_vec())
            .collect::<Vec<_>>()
    };
    let _ = tls.shutdown().await;
    Ok(chain)
}

#[derive(Debug)]
struct CaptureVerifier;

impl ServerCertVerifier for CaptureVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}

// ── URL + PEM formatting helpers ─────────────────────────────────────────────

struct ParsedUrl {
    host: String,
    port: u16,
}

fn parse_url(url: &str) -> Option<ParsedUrl> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().ok()?),
        None => (authority.to_string(), 443u16),
    };
    if host.is_empty() {
        return None;
    }
    Some(ParsedUrl { host, port })
}

fn der_to_pem(der: &[u8]) -> String {
    let encoded = base64_encode(der);
    let mut out = String::with_capacity(encoded.len() + 64);
    out.push_str("-----BEGIN CERTIFICATE-----\n");
    for chunk in encoded.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap());
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16)
            | ((input[i + 1] as u32) << 8)
            | (input[i + 2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}
