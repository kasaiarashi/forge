// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use x509_parser::prelude::*;

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

    // If a pin already exists, validate it against the server's *current*
    // certificate before trusting it. A bare `has_pin` check would
    // silently return Ok here even when forge-server has regenerated its
    // CA (data dir wiped, reinstall, etc.), pushing the failure downstream
    // to the real gRPC handshake as a cryptic "invalid peer certificate:
    // BadSignature". Instead: handshake-test the pin, and if it no longer
    // validates, delete it and fall through to re-TOFU so the user gets
    // the normal fingerprint-confirmation flow.
    if has_pin(server_url) {
        if pinned_handshake_ok(server_url).await {
            return Ok(());
        }
        if let Ok(p) = pin_path(server_url) {
            let _ = std::fs::remove_file(&p);
        }
        eprintln!(
            "warning: the stored TLS pin for {server_url} no longer matches \
             the server's certificate."
        );
        eprintln!(
            "         this usually means forge-server regenerated its CA. \
             re-verifying trust..."
        );
        eprintln!();
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
        .with_context(|| format!("could not reach {server_url} to inspect its TLS certificate"))?;
    if chain.is_empty() {
        bail!("server at {server_url} presented no certificates");
    }

    // The CA is the last entry in the chain (forge-server sends leaf + CA);
    // the leaf is the first. If the server only sent a leaf we pin the leaf,
    // which means the user has to re-trust on leaf rotation. forge-server's
    // auto-gen always sends both, so the CA path is the common case.
    let leaf = chain.first().unwrap();
    let anchor = chain.last().unwrap();

    print_cert_details(server_url, leaf, anchor);

    if auto_yes {
        eprintln!("Auto-trusting (non-interactive or --yes).");
    } else if !io::stdin().is_terminal() {
        bail!(
            "cannot prompt for trust: stdin is not a terminal. Re-run with \
             --yes to accept the fingerprint automatically, or run \
             `forge trust {server_url}` first."
        );
    } else if !prompt_yes_no("Trust this certificate and continue? [y/N] ")? {
        bail!("trust declined; aborting");
    }

    write_pin(server_url, anchor)?;
    let pin = pin_path(server_url)?;
    println!("Pinned to {}.", pin.display());

    // Ask whether to also push this CA into the OS trust store so
    // browsers, curl, and other system tools stop warning about it. The
    // prompt is skipped for `--yes` / non-TTY runs because installing
    // into a machine-wide store is something we don't want to do silently
    // from an automated script.
    if !auto_yes && io::stdin().is_terminal() {
        let default_install = is_ca_anchor(anchor);
        let prompt = if default_install {
            "Also install this certificate into your OS trust store so other apps trust it? [Y/n] "
        } else {
            "Also install this certificate into your OS trust store so other apps trust it? [y/N] "
        };
        if prompt_yes_no_default(prompt, default_install)? {
            match install_into_os_trust_store(&pin) {
                Ok(msg) => println!("{msg}"),
                Err(e) => {
                    eprintln!("Could not install into OS trust store: {e:#}");
                    eprintln!(
                        "The pin at {} is enough for the forge CLI itself — this is \
                         only a convenience for browsers / curl / other tools.",
                        pin.display()
                    );
                }
            }
        }
    }

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

/// Render the peer chain as a short operator-readable report: subject /
/// issuer DN parts (CN, O, OU), validity window, and SHA-256 fingerprints
/// for both the leaf and the anchor. Parse failures degrade gracefully —
/// we still show the raw fingerprints so the operator can always verify
/// out-of-band.
fn print_cert_details(url: &str, leaf: &[u8], anchor: &[u8]) {
    let leaf_fp = sha256_fingerprint(leaf);
    let anchor_fp = sha256_fingerprint(anchor);

    println!();
    println!("This is the first time connecting to {url} and the server's");
    println!("certificate isn't in your system trust store.");
    println!();

    // Always show the anchor (what we're going to pin) first, so the
    // operator compares the right fingerprint against the server log.
    println!("  \x1b[1mCertificate Authority (trust anchor)\x1b[0m");
    if let Some(info) = parse_cert_info(anchor) {
        print_info_block(&info, 4);
    } else {
        println!("    (could not parse certificate — showing fingerprint only)");
    }
    println!("    SHA-256:    {anchor_fp}");
    println!();

    if leaf_fp != anchor_fp {
        println!("  \x1b[1mLeaf certificate (what the server presented)\x1b[0m");
        if let Some(info) = parse_cert_info(leaf) {
            print_info_block(&info, 4);
        }
        println!("    SHA-256:    {leaf_fp}");
        println!();
    }

    println!("Compare the CA SHA-256 above against the fingerprint the server operator");
    println!("printed on startup (look for 'TLS CA fingerprint' in the forge-server logs)");
    println!("before accepting.");
    println!();
}

/// Minimal cert summary we pull out for display.
struct CertInfo {
    subject: String,
    issuer: String,
    not_before: String,
    not_after: String,
}

fn parse_cert_info(der: &[u8]) -> Option<CertInfo> {
    let (_, cert) = X509Certificate::from_der(der).ok()?;
    Some(CertInfo {
        subject: render_name(cert.subject()),
        issuer: render_name(cert.issuer()),
        not_before: cert.validity().not_before.to_string(),
        not_after: cert.validity().not_after.to_string(),
    })
}

/// Render an X.500 Name as `CN=Foo, O=Bar, OU=Baz`. Skips blank RDNs
/// and is resilient to certs that omit common fields.
fn render_name(name: &X509Name<'_>) -> String {
    let mut parts: Vec<String> = Vec::new();
    for rdn in name.iter() {
        for attr in rdn.iter() {
            let label = match attr.attr_type().to_string().as_str() {
                "2.5.4.3" => "CN",
                "2.5.4.10" => "O",
                "2.5.4.11" => "OU",
                "2.5.4.6" => "C",
                "2.5.4.7" => "L",
                "2.5.4.8" => "ST",
                _ => continue,
            };
            if let Ok(val) = attr.as_str() {
                parts.push(format!("{label}={val}"));
            }
        }
    }
    if parts.is_empty() {
        "(empty)".into()
    } else {
        parts.join(", ")
    }
}

fn print_info_block(info: &CertInfo, indent: usize) {
    let pad = " ".repeat(indent);
    println!("{pad}Subject:    {}", info.subject);
    println!("{pad}Issuer:     {}", info.issuer);
    println!("{pad}Valid from: {}", info.not_before);
    println!("{pad}Valid to:   {}", info.not_after);
}

/// Heuristic: treat an anchor as a CA worth installing machine-wide if
/// it has the CA basic constraint set. Leaf-only pins fall back to a
/// `[y/N]` default because installing a leaf into Root is almost never
/// what the user wants.
fn is_ca_anchor(der: &[u8]) -> bool {
    match X509Certificate::from_der(der) {
        Ok((_, cert)) => cert
            .basic_constraints()
            .ok()
            .flatten()
            .map(|ext| ext.value.ca)
            .unwrap_or(false),
        Err(_) => false,
    }
}

fn prompt_yes_no(prompt: &str) -> Result<bool> {
    prompt_yes_no_default(prompt, false)
}

fn prompt_yes_no_default(prompt: &str, default_yes: bool) -> Result<bool> {
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let trimmed = line.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Ok(default_yes);
    }
    Ok(matches!(trimmed.as_str(), "y" | "yes"))
}

// ── OS trust store install ──────────────────────────────────────────────

/// Install the pinned PEM into the OS-level trust store. Platform-
/// specific. Returns a human-readable success message on success.
fn install_into_os_trust_store(pem_path: &Path) -> Result<String> {
    #[cfg(windows)]
    {
        install_windows(pem_path)
    }
    #[cfg(target_os = "macos")]
    {
        install_macos(pem_path)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        install_linux(pem_path)
    }
}

/// Detect whether the current process is running with an elevated
/// administrator token. Failure to probe is treated as "not elevated",
/// which is the safer default — we'll re-launch via UAC.
#[cfg(windows)]
fn is_elevated() -> bool {
    use std::process::Command;
    // `net session` returns exit 0 only for members of the Administrators
    // group running with an elevated token. It's a well-known idiom and
    // avoids pulling a whole Win32 crate just to read TOKEN_ELEVATION.
    Command::new("net")
        .args(["session"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn install_windows(pem_path: &Path) -> Result<String> {
    use std::process::Command;
    let path_str = pem_path.to_string_lossy().to_string();

    if is_elevated() {
        let out = Command::new("certutil")
            .args(["-addstore", "-f", "Root", &path_str])
            .output()
            .context("running certutil")?;
        if out.status.success() {
            return Ok(format!(
                "Installed into Windows LocalMachine\\Root (machine-wide)."
            ));
        }
        bail!(
            "certutil exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    // Non-elevated: re-launch certutil via PowerShell's `Start-Process
    // -Verb RunAs`, which pops the UAC consent dialog. `-Wait` blocks
    // until the child exits so we know whether the install succeeded.
    println!("Requesting administrator elevation (you'll see a UAC prompt)...");

    // Escape single quotes in the path (rare but possible).
    let ps_path = path_str.replace('\'', "''");
    let ps = format!(
        "try {{ $p = Start-Process -FilePath 'certutil.exe' \
         -ArgumentList @('-addstore','-f','Root','{ps_path}') \
         -Verb RunAs -Wait -PassThru -WindowStyle Hidden; \
         exit $p.ExitCode }} catch {{ exit 1 }}"
    );

    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .status()
        .context("launching elevated powershell")?;

    if status.success() {
        Ok("Installed into Windows LocalMachine\\Root (machine-wide).".into())
    } else {
        bail!(
            "elevated certutil was cancelled or failed (exit {}). You can \
             re-run from an Administrator PowerShell: \
             `certutil -addstore -f Root \"{path_str}\"`",
            status.code().unwrap_or(-1)
        )
    }
}

#[cfg(target_os = "macos")]
fn install_macos(pem_path: &Path) -> Result<String> {
    use std::process::Command;
    let out = Command::new("security")
        .args([
            "add-trusted-cert",
            "-d",
            "-r",
            "trustRoot",
            "-k",
            "/Library/Keychains/System.keychain",
        ])
        .arg(pem_path)
        .output()
        .context("running /usr/bin/security")?;
    if out.status.success() {
        Ok("Installed into /Library/Keychains/System.keychain (machine-wide).".into())
    } else {
        bail!(
            "security exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn install_linux(pem_path: &Path) -> Result<String> {
    // The portable Linux recipe — copy into /usr/local/share/ca-certificates
    // and run update-ca-certificates — needs root. Rather than trying to
    // shell out to sudo from here (which would surprise users), we print
    // the one-liner they should run.
    let p = pem_path.display();
    Ok(format!(
        "To trust this CA system-wide on Linux, run:\n  \
         sudo cp {p} /usr/local/share/ca-certificates/forge-vcs-ca.crt && \
         sudo update-ca-certificates"
    ))
}

fn has_pin(server_url: &str) -> bool {
    pin_path(server_url).map(|p| p.exists()).unwrap_or(false)
}

fn pin_path(server_url: &str) -> Result<PathBuf> {
    let parsed = parse_url(server_url).ok_or_else(|| anyhow!("unparseable URL: {server_url}"))?;
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
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
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

/// Try a strict TLS handshake using ONLY the pinned certificate as the
/// trust anchor. Returns `true` when the stored pin still validates the
/// server's current cert, `false` on any failure (I/O, parse, TLS). The
/// caller (`ensure_trusted`) uses `false` as the "pin is stale, re-TOFU"
/// signal, so we deliberately swallow every error type here rather than
/// distinguishing them — the recovery path is identical either way.
async fn pinned_handshake_ok(server_url: &str) -> bool {
    let parsed = match parse_url(server_url) {
        Some(p) => p,
        None => return false,
    };
    let pem = match pin_path(server_url)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
    {
        Some(p) => p,
        None => return false,
    };

    let mut roots = rustls::RootCertStore::empty();
    let mut cursor = std::io::Cursor::new(pem);
    for cert in rustls_pemfile::certs(&mut cursor) {
        match cert {
            Ok(c) => {
                if roots.add(c).is_err() {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
    if roots.is_empty() {
        return false;
    }

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

/// Recovery entry point invoked by `client::connect_*` when a real gRPC
/// handshake fails with a certificate-verification error *and* a pin is
/// present. We assume the pin has gone stale (forge-server regenerated
/// its CA after a data-dir wipe / reinstall), delete it, and run the
/// normal TOFU flow so the user can re-verify the fingerprint
/// interactively. The caller retries its connect afterward — the freshly
/// written pin is picked up automatically on `build_endpoint`'s next
/// call because `load_pinned_trust` re-reads the file.
pub async fn reverify_after_cert_mismatch(server_url: &str) -> Result<()> {
    eprintln!();
    eprintln!("The server's TLS certificate does not match the stored pin for");
    eprintln!("{server_url}.");
    eprintln!();
    eprintln!("This usually means forge-server regenerated its self-signed CA");
    eprintln!("(the data directory was wiped, or the server was reinstalled).");
    eprintln!();
    eprintln!("Re-verifying trust...");
    eprintln!();
    if let Ok(p) = pin_path(server_url) {
        let _ = std::fs::remove_file(&p);
    }
    ensure_trusted(server_url, false).await
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
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
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
