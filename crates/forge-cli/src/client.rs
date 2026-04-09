// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Centralized gRPC client factory for the `forge` CLI.
//!
//! Every command that talks to the server should call [`connect_forge`] (or
//! [`connect_auth`]) instead of `ForgeServiceClient::connect` directly. The
//! factory:
//!
//! 1. Loads the stored credential for the target server (env > keychain >
//!    file — see [`crate::credentials`]).
//! 2. Opens the gRPC channel.
//! 3. Wraps the client in a tonic interceptor that injects
//!    `Authorization: Bearer <token>` on every outgoing call.
//!
//! When no credential is found the request still goes out, just without an
//! Authorization header. The server treats that as `Caller::Anonymous` and
//! the per-handler authz check decides whether to allow it (only for read on
//! a public repo).

use anyhow::{Context, Result};
use forge_proto::forge::auth_service_client::AuthServiceClient;
use forge_proto::forge::forge_service_client::ForgeServiceClient;
use tonic::metadata::MetadataValue;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};

use crate::credentials::{self, Credential};
use crate::url_resolver;

/// Build a tonic `Endpoint` for `server_url`, wiring TLS when the URL is
/// `https://…`. Trust resolution order (first match wins):
///
/// 1. `FORGE_CA_CERT` env var → PEM path. Useful for one-off testing.
/// 2. `~/.forge/trusted/<host>_<port>.pem` → pinned by `forge trust`.
/// 3. System trust store (webpki-roots / OS root store via rustls-native-certs).
///
/// The pinned-trust layer is why `forge login https://…` "just works" after
/// a one-time `forge trust https://…`.
fn build_endpoint(server_url: &str) -> Result<Endpoint> {
    let endpoint = Endpoint::from_shared(server_url.to_string())
        .with_context(|| format!("invalid server url '{server_url}'"))?;
    if !server_url.starts_with("https://") {
        return Ok(endpoint);
    }

    // 1) FORGE_CA_CERT env var takes priority — operator override.
    if let Ok(path) = std::env::var("FORGE_CA_CERT") {
        let pem = std::fs::read(&path)
            .with_context(|| format!("failed to read FORGE_CA_CERT={path}"))?;
        let tls = ClientTlsConfig::new().ca_certificate(Certificate::from_pem(pem));
        return endpoint
            .tls_config(tls)
            .with_context(|| format!("tls config for {server_url}"));
    }

    // 2) Pinned trust anchor saved by `forge trust`.
    if let Some(pinned) = load_pinned_trust(server_url) {
        let tls = ClientTlsConfig::new().ca_certificate(Certificate::from_pem(pinned));
        return endpoint
            .tls_config(tls)
            .with_context(|| format!("tls config for {server_url}"));
    }

    // 3) System trust (for real PKI deployments).
    let tls = ClientTlsConfig::new().with_native_roots();
    endpoint
        .tls_config(tls)
        .with_context(|| format!("tls config for {server_url}"))
}

/// Look up a `~/.forge/trusted/<host>_<port>.pem` that matches the host
/// and port in `server_url`. Returns the raw PEM bytes on match, `None` on
/// anything else — we never fail loudly here because a cache miss should
/// just fall through to the next trust layer.
fn load_pinned_trust(server_url: &str) -> Option<Vec<u8>> {
    let rest = server_url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().ok()?),
        None => (authority.to_string(), 443u16),
    };
    let sanitized: String = host
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => c,
            _ => '_',
        })
        .collect();
    let home = dirs::home_dir()?;
    let path = home
        .join(".forge")
        .join("trusted")
        .join(format!("{sanitized}_{port}.pem"));
    std::fs::read(path).ok()
}

/// Build a fresh `ForgeServiceClient` against `server_url`, attaching the
/// stored credential as an `Authorization` header on every call.
pub async fn connect_forge(
    server_url: &str,
) -> Result<ForgeServiceClient<InterceptedService<Channel, AuthInterceptor>>> {
    let (channel, interceptor) = connect_with_auth(server_url).await?;
    Ok(ForgeServiceClient::with_interceptor(channel, interceptor))
}

/// Same as [`connect_forge`] but for the `AuthService` (used by login,
/// whoami, PAT mint, etc.). Auth headers are still injected so that
/// authenticated AuthService methods like `WhoAmI` work.
pub async fn connect_auth(
    server_url: &str,
) -> Result<AuthServiceClient<InterceptedService<Channel, AuthInterceptor>>> {
    let (channel, interceptor) = connect_with_auth(server_url).await?;
    Ok(AuthServiceClient::with_interceptor(channel, interceptor))
}

/// Connect to the `AuthService` with NO stored credential attached.
///
/// Used by `forge login` itself: the user might have a stale PAT in their
/// keychain (e.g. the server's DB was reset, or the token was revoked). If
/// we forward that stale PAT on the Login RPC, forge-server's interceptor
/// rejects it as "invalid or revoked token" before the login handler ever
/// runs — making it impossible to log in again without manually clearing
/// the keychain. An anonymous client sidesteps that.
pub async fn connect_auth_anonymous(server_url: &str) -> Result<AuthServiceClient<Channel>> {
    // Resolve web-UI URLs to the underlying gRPC URL automatically. See
    // `url_resolver` for the threat-model rationale (probe is lenient
    // TLS; actual gRPC connect still enforces pinned trust below).
    let server_url = url_resolver::resolve(server_url).await;
    let endpoint = build_endpoint(&server_url)?;
    let channel = endpoint
        .connect()
        .await
        .map_err(|e| format_connect_error(&server_url, e))?;
    Ok(AuthServiceClient::new(channel))
}

async fn connect_with_auth(server_url: &str) -> Result<(Channel, AuthInterceptor)> {
    let server_url = url_resolver::resolve(server_url).await;
    // Credentials are keyed by the RESOLVED URL so that a subsequent
    // `forge push` — which starts from a workspace origin pointing at the
    // web URL — finds the same token that `forge login` stored.
    let cred = credentials::load(&server_url)?;
    let endpoint = build_endpoint(&server_url)?;
    let channel = endpoint
        .connect()
        .await
        .map_err(|e| format_connect_error(&server_url, e))?;
    Ok((channel, AuthInterceptor::new(cred)))
}

/// Produce a connect-error message that includes the full `std::error::Error`
/// source chain. tonic's `transport::Error` prints "transport error" at the
/// top level and hides the actual cause (TLS verification, SAN mismatch,
/// connection refused, etc.) behind `.source()`. We walk the chain by hand
/// so the user sees what's actually wrong rather than a generic "connect".
fn format_connect_error(url: &str, err: tonic::transport::Error) -> anyhow::Error {
    let mut msg = format!("connect to forge server at {url}: {err}");
    let mut src: Option<&dyn std::error::Error> = std::error::Error::source(&err);
    while let Some(s) = src {
        msg.push_str("\n  caused by: ");
        msg.push_str(&s.to_string());
        src = s.source();
    }
    anyhow::anyhow!(msg)
}

/// tonic interceptor closure that injects the bearer token. We don't use a
/// raw closure because tonic's `InterceptedService` requires the interceptor
/// to be `Clone + Send + Sync + 'static`, and a generic closure can't carry
/// the cached `MetadataValue` cleanly.
#[derive(Clone)]
pub struct AuthInterceptor {
    header: Option<MetadataValue<tonic::metadata::Ascii>>,
}

impl AuthInterceptor {
    fn new(cred: Option<Credential>) -> Self {
        let header = cred.and_then(|c| {
            let raw = format!("Bearer {}", c.token);
            MetadataValue::try_from(raw).ok()
        });
        Self { header }
    }
}

impl tonic::service::Interceptor for AuthInterceptor {
    fn call(
        &mut self,
        mut request: tonic::Request<()>,
    ) -> Result<tonic::Request<()>, tonic::Status> {
        if let Some(ref h) = self.header {
            request.metadata_mut().insert("authorization", h.clone());
        }
        Ok(request)
    }
}
