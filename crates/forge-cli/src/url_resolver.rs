// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! "Did you mean the forge-server URL?" auto-resolver.
//!
//! Users constantly paste the forge-web URL (port 3000, browser UI) into
//! `forge login --server …` because that's the URL they see in their
//! address bar. The CLI actually needs the forge-server gRPC URL (port
//! 9876). Instead of failing with "connect error", we try to detect the
//! mistake and transparently switch.
//!
//! # How it works
//!
//! 1. Every CLI command that takes a server URL calls [`resolve`] first.
//! 2. [`resolve`] opens a TLS connection to that URL with a "trust nothing
//!    but capture the handshake" verifier (same machinery as `forge
//!    trust`), then sends a raw HTTP/1.1 GET for
//!    `/.well-known/forge-server-info`.
//! 3. If the peer responds with `{"service":"forge-web","grpc_scheme":…,
//!    "grpc_port":…}`, we rebuild the URL using the original host plus the
//!    advertised scheme + port and return that.
//! 4. If anything fails (not HTTPS, no well-known endpoint, non-JSON
//!    body, parse error), we return the original URL unchanged — the
//!    URL was probably already a forge-server gRPC endpoint, or the
//!    server is unreachable, in which case the downstream gRPC call will
//!    surface the real error anyway.
//!
//! # Trust model
//!
//! The probe uses a permissive TLS verifier. This is safe because the
//! probe only reads a public JSON blob containing a port number — no
//! credentials flow in or out. The port number is then fed into the real
//! gRPC connect path which **does** enforce certificate pinning (via
//! `~/.forge/trusted/` or `FORGE_CA_CERT` or the system trust store), so a
//! MITM on the probe can't silently redirect the user to a rogue server —
//! the subsequent `forge trust` / gRPC handshake would refuse to talk to
//! an unpinned / invalid cert.

use anyhow::{anyhow, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

/// In-memory cache keyed by the raw URL string. Populated on first
/// resolution per `forge` invocation so we don't re-probe on every gRPC
/// call within the same command.
fn cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve `url` to a forge-server gRPC URL. When `url` already points at
/// forge-server, returns it unchanged. When it points at forge-web, prints
/// a notice and returns the detected gRPC URL.
pub async fn resolve(url: &str) -> String {
    // Fast path: cached.
    if let Some(cached) = cache().lock().unwrap().get(url).cloned() {
        return cached;
    }

    let resolved = resolve_once(url).await.unwrap_or_else(|_| url.to_string());

    // Cache even negative hits (the probe failed → we keep the original).
    cache()
        .lock()
        .unwrap()
        .insert(url.to_string(), resolved.clone());
    resolved
}

async fn resolve_once(url: &str) -> Result<String> {
    let parsed = parse_url(url).ok_or_else(|| anyhow!("unparseable URL"))?;
    // Only probe https:// for now. http:// works in theory, but forge-web
    // no longer serves plaintext by default and the extra code isn't worth it.
    if parsed.scheme != "https" {
        return Ok(url.to_string());
    }

    // Short timeout so a genuinely unreachable server doesn't block the CLI.
    let body = match tokio::time::timeout(
        Duration::from_secs(2),
        fetch_well_known(&parsed.host, parsed.port),
    )
    .await
    {
        Ok(Ok(b)) => b,
        _ => return Ok(url.to_string()),
    };

    // Shallow JSON parse — we only need two fields, and pulling in a full
    // JSON dep just for this would be overkill. The response is server-
    // generated, so it's well-formed.
    let scheme = extract_str_field(&body, "grpc_scheme").unwrap_or_else(|| "https".to_string());
    let port = extract_num_field(&body, "grpc_port").ok_or_else(|| anyhow!("no grpc_port"))?;
    let service = extract_str_field(&body, "service").unwrap_or_default();
    if service != "forge-web" {
        return Ok(url.to_string());
    }

    let new_url = format!("{scheme}://{}:{port}", parsed.host);
    if new_url != url {
        eprintln!(
            "note: {url} is a forge-web UI; using gRPC endpoint {new_url}",
        );
    }
    Ok(new_url)
}

// ── URL parsing ──────────────────────────────────────────────────────────────

struct ParsedUrl {
    scheme: String,
    host: String,
    port: u16,
}

fn parse_url(url: &str) -> Option<ParsedUrl> {
    let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = url.strip_prefix("http://") {
        ("http", r)
    } else {
        return None;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().ok()?),
        None => (
            authority.to_string(),
            if scheme == "https" { 443 } else { 80 },
        ),
    };
    if host.is_empty() {
        return None;
    }
    Some(ParsedUrl {
        scheme: scheme.to_string(),
        host,
        port,
    })
}

// ── Permissive TLS + raw HTTP/1.1 GET ────────────────────────────────────────

#[derive(Debug)]
struct ProbeVerifier;

impl ServerCertVerifier for ProbeVerifier {
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

async fn fetch_well_known(host: &str, port: u16) -> Result<String> {
    let tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(ProbeVerifier))
        .with_no_client_auth();
    let connector = TlsConnector::from(std::sync::Arc::new(tls_config));

    let stream = TcpStream::connect((host, port)).await?;
    let sname = ServerName::try_from(host.to_string())
        .map_err(|e| anyhow!("invalid server name: {e}"))?;
    let mut tls = connector.connect(sname, stream).await?;

    // Raw HTTP/1.1. forge-web is axum (hyper) and speaks HTTP/1.1 just
    // fine alongside HTTP/2 for browsers. `Connection: close` tells the
    // server we only want one request so it closes after the body.
    let req = format!(
        "GET /.well-known/forge-server-info HTTP/1.1\r\n\
         Host: {host}\r\n\
         User-Agent: forge-cli\r\n\
         Accept: application/json\r\n\
         Connection: close\r\n\
         \r\n"
    );
    tls.write_all(req.as_bytes()).await?;
    tls.flush().await?;

    let mut raw = Vec::with_capacity(512);
    tls.read_to_end(&mut raw).await?;

    // Split headers from body at the first \r\n\r\n.
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("no header/body separator in response"))?;
    let status_line = std::str::from_utf8(&raw[..raw[..sep].iter().position(|&b| b == b'\r').unwrap_or(sep)])
        .unwrap_or("");
    if !status_line.contains("200") {
        return Err(anyhow!("well-known returned non-200: {status_line}"));
    }

    // The body may be chunked — we don't want to pull a full HTTP parser
    // in, so we accept either an identity body or a single-chunk
    // transfer-encoding. In practice axum with `Json(...)` returns
    // Content-Length + identity, so the simple slice is usually correct.
    // If chunked, the first line inside the body is the hex length; we
    // just look for the opening `{` and trailing `}` since the payload is
    // small and JSON is bracketed.
    let body_bytes = &raw[sep + 4..];
    let body_text = std::str::from_utf8(body_bytes).map_err(|_| anyhow!("non-utf8 body"))?;
    let open = body_text
        .find('{')
        .ok_or_else(|| anyhow!("no JSON object in body"))?;
    let close = body_text
        .rfind('}')
        .ok_or_else(|| anyhow!("no JSON terminator in body"))?;
    Ok(body_text[open..=close].to_string())
}

// ── Micro JSON field extraction ──────────────────────────────────────────────
//
// The well-known response is a flat JSON object with two or three string
// and number fields. A full serde_json parse would work but pulls nothing
// new in here — we already have it in forge-cli via tonic. Using it would
// still work; we stay hand-rolled because the shape is frozen and we want
// to keep this module self-contained.

fn extract_str_field(body: &str, field: &str) -> Option<String> {
    let key = format!("\"{field}\"");
    let start = body.find(&key)?;
    let after = &body[start + key.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_num_field(body: &str, field: &str) -> Option<u16> {
    let key = format!("\"{field}\"");
    let start = body.find(&key)?;
    let after = &body[start + key.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}
