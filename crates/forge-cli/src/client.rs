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
use tonic::transport::{Channel, Endpoint};

use crate::credentials::{self, Credential};

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
    let endpoint = Endpoint::from_shared(server_url.to_string())
        .with_context(|| format!("invalid server url '{server_url}'"))?;
    let channel = endpoint
        .connect()
        .await
        .with_context(|| format!("connect to forge server at {server_url}"))?;
    Ok(AuthServiceClient::new(channel))
}

async fn connect_with_auth(server_url: &str) -> Result<(Channel, AuthInterceptor)> {
    let cred = credentials::load(server_url)?;
    let endpoint = Endpoint::from_shared(server_url.to_string())
        .with_context(|| format!("invalid server url '{server_url}'"))?;
    let channel = endpoint
        .connect()
        .await
        .with_context(|| format!("connect to forge server at {server_url}"))?;
    Ok((channel, AuthInterceptor::new(cred)))
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
