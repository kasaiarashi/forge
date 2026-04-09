// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! gRPC tonic interceptor that turns an `Authorization: Bearer <token>`
//! header into a [`Caller`] and stashes it in the request extensions.
//!
//! The interceptor only does *authentication* — it answers "who is this?".
//! Per-handler *authorization* ("is this caller allowed to do X on repo Y?")
//! lives in [`super::authorize`] and is invoked by each handler.
//!
//! # Token resolution
//!
//! 1. No `Authorization` header → `Caller::Anonymous`. The handler's authz
//!    check decides whether anonymous is acceptable (only for read on a
//!    public repo).
//! 2. Present-but-malformed header → `Status::unauthenticated`. We never
//!    silently downgrade a malformed bearer to anonymous.
//! 3. Bearer token starting with `fpat_` → look up in
//!    `personal_access_tokens`. PATs are the common case from the CLI so we
//!    check them first.
//! 4. Bearer token starting with `fses_` → look up in `sessions`.
//! 5. Either lookup miss → `Status::unauthenticated`.
//!
//! Both lookups use the indexed `token_prefix` column for an O(1) candidate
//! list and then verify the argon2id hash. There's no in-memory cache yet —
//! that's a phase 7 optimization once we measure real load.

use std::sync::Arc;
use tonic::{Request, Status};

use super::caller::{AuthenticatedCaller, Caller, CredentialKind};
use super::store::UserStore;
use super::tokens::{PAT_PREFIX, SESSION_PREFIX};

/// Build the tonic interceptor closure used by the gRPC server.
///
/// The returned closure is `Clone + Send + Sync + 'static` so it can be
/// installed on a tonic `InterceptedService` and shared across worker
/// threads.
pub fn make_interceptor(
    store: Arc<dyn UserStore>,
) -> impl Fn(Request<()>) -> Result<Request<()>, Status> + Clone + Send + Sync + 'static {
    move |mut req: Request<()>| {
        let header = req
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        let caller = match header.as_deref() {
            None => Caller::anonymous(),
            Some(raw) => {
                let token = raw
                    .strip_prefix("Bearer ")
                    .or_else(|| raw.strip_prefix("bearer "))
                    .ok_or_else(|| {
                        Status::unauthenticated("Authorization header must be 'Bearer <token>'")
                    })?;
                resolve_token(store.as_ref(), token)?
            }
        };

        req.extensions_mut().insert(caller);
        Ok(req)
    }
}

fn resolve_token(store: &dyn UserStore, plaintext: &str) -> Result<Caller, Status> {
    if plaintext.starts_with(PAT_PREFIX) {
        match store
            .find_pat_by_plaintext(plaintext)
            .map_err(|e| Status::internal(format!("token lookup failed: {e}")))?
        {
            Some((pat, user)) => {
                // Best-effort touch — don't fail the request if it errors.
                let _ = store.touch_pat(pat.id);
                Ok(Caller::Authenticated(AuthenticatedCaller {
                    user_id: user.id,
                    username: user.username,
                    is_server_admin: user.is_server_admin,
                    scopes: pat.scopes,
                    credential: CredentialKind::PersonalAccessToken,
                }))
            }
            None => Err(Status::unauthenticated("invalid or revoked token")),
        }
    } else if plaintext.starts_with(SESSION_PREFIX) {
        match store
            .find_session_by_plaintext(plaintext)
            .map_err(|e| Status::internal(format!("session lookup failed: {e}")))?
        {
            Some((session, user)) => {
                let _ = store.touch_session(session.id);
                Ok(Caller::Authenticated(AuthenticatedCaller {
                    user_id: user.id,
                    username: user.username,
                    is_server_admin: user.is_server_admin,
                    scopes: vec![], // sessions are unscoped — Caller::has_scope short-circuits
                    credential: CredentialKind::Session,
                }))
            }
            None => Err(Status::unauthenticated("invalid or expired session")),
        }
    } else {
        Err(Status::unauthenticated("unrecognized token format"))
    }
}

/// Extract the [`Caller`] previously inserted by [`make_interceptor`]. Every
/// gRPC handler should call this on its incoming request before doing any
/// real work. Returns owned [`Caller`] (cheap clone — anonymous is a unit
/// variant, authenticated only clones a username + a small Vec of scopes).
///
/// If the interceptor isn't installed (should never happen in production but
/// can happen in tests), this returns [`Caller::Anonymous`] rather than
/// panicking.
pub fn caller_of<T>(request: &Request<T>) -> Caller {
    request
        .extensions()
        .get::<Caller>()
        .cloned()
        .unwrap_or(Caller::Anonymous)
}
