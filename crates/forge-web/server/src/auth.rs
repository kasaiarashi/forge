// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Web UI auth shim.
//!
//! This module owns **no** auth state of its own — it's a thin HTTP wrapper
//! around `forge-server`'s `AuthService`. Browser sends username+password to
//! `/api/auth/login`, we forward it to `AuthService::Login` over gRPC, get
//! back a session token, set it as an HttpOnly cookie. Subsequent requests
//! carry the cookie back, the [`session_token_layer`] middleware extracts
//! it, stashes it in a tokio task-local, and the gRPC client reads it back
//! out when constructing per-request bearer-attached client connections.
//!
//! No bcrypt, no JWT, no admin-password-hash in TOML — all of that lives in
//! forge-server now.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use forge_proto::forge::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;

const COOKIE_NAME: &str = "forge_session";

tokio::task_local! {
    /// The web request's session token, scoped to the lifetime of one
    /// request via [`session_token_layer`]. Read by [`crate::AppState::grpc_client`]
    /// to attach the right Authorization header to upstream gRPC calls.
    pub static SESSION_TOKEN: Option<String>;
}

// ── Middleware ───────────────────────────────────────────────────────────────

/// Axum middleware that extracts the session cookie and runs the rest of the
/// request inside a task-local scope so downstream handlers can reach it via
/// [`current_session_token`].
pub async fn session_token_layer(req: Request<Body>, next: Next) -> Response {
    let token = extract_cookie_token(&req);
    SESSION_TOKEN.scope(token, async move { next.run(req).await }).await
}

/// Returns the session token for the current request, or `None` if there is
/// no cookie. Safe to call from any handler running underneath
/// [`session_token_layer`].
pub fn current_session_token() -> Option<String> {
    SESSION_TOKEN.try_with(|t| t.clone()).unwrap_or(None)
}

fn extract_cookie_token(req: &Request<Body>) -> Option<String> {
    let header = req.headers().get(header::COOKIE)?.to_str().ok()?;
    for part in header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(&format!("{COOKIE_NAME}=")) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

// ── HTTP DTOs ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct UserDto {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub is_server_admin: bool,
}

#[derive(Debug, Serialize)]
pub struct LoginOk {
    /// Kept for backward compatibility with the existing SPA login form which
    /// checks for `ok: true` to decide whether the call succeeded.
    pub ok: bool,
    /// Kept for backward compatibility — the SPA reads this top-level
    /// `username` field directly. New SPA code should prefer `user.username`.
    pub username: String,
    pub user: UserDto,
}

#[derive(Debug, Serialize)]
pub struct WhoAmIDto {
    /// Top-level fields the existing SPA reads directly. Do not remove
    /// without coordinating with `crates/forge-web/ui/src/api.ts`'s `User`
    /// interface.
    pub username: String,
    pub is_admin: bool,

    // Newer shape used by post-rewrite SPA pages (token mgmt, sessions, etc.)
    pub authenticated: bool,
    pub user: Option<UserDto>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct InitializedDto {
    pub initialized: bool,
}

#[derive(Debug, Deserialize)]
pub struct BootstrapBody {
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
}

fn err(status: StatusCode, msg: &str) -> Response {
    (
        status,
        Json(ErrorBody {
            error: msg.to_string(),
        }),
    )
        .into_response()
}

fn user_dto(u: &UserInfo) -> UserDto {
    UserDto {
        id: u.id,
        username: u.username.clone(),
        email: u.email.clone(),
        display_name: u.display_name.clone(),
        is_server_admin: u.is_server_admin,
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `POST /api/auth/login` — username + password in, session cookie out.
///
/// Uses the anonymous gRPC client so a user who already has a stale or
/// expired `forge_session` cookie can still log in. If we forwarded the
/// cookie here, forge-server's bearer interceptor would reject it as
/// "invalid or expired session" before our handler ever ran.
pub async fn login(State(state): State<Arc<AppState>>, Json(body): Json<LoginBody>) -> Response {
    let mut auth = match state.grpc_auth_client_anonymous().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };

    let resp = match auth
        .login(LoginRequest {
            username: body.username,
            password: body.password,
            user_agent: "forge-web".to_string(),
            ip: String::new(),
        })
        .await
    {
        Ok(r) => r.into_inner(),
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            return err(StatusCode::UNAUTHORIZED, s.message());
        }
        Err(s) => return err(StatusCode::BAD_GATEWAY, s.message()),
    };

    let user = match resp.user {
        Some(u) => u,
        None => return err(StatusCode::BAD_GATEWAY, "no user in response"),
    };

    let max_age = (resp.expires_at - chrono::Utc::now().timestamp()).max(0);
    let cookie = format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}",
        token = resp.session_token,
        max_age = max_age
    );

    let dto = user_dto(&user);
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(LoginOk {
            ok: true,
            username: dto.username.clone(),
            user: dto,
        }),
    )
        .into_response()
}

/// `POST /api/auth/logout` — best-effort revoke + clear cookie.
pub async fn logout(State(state): State<Arc<AppState>>) -> Response {
    if let Ok(mut auth) = state.grpc_auth_client().await {
        let _ = auth.logout(LogoutRequest {}).await;
    }
    let clear = format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
    (
        StatusCode::OK,
        [(header::SET_COOKIE, clear)],
        Json(serde_json::json!({"ok": true})),
    )
        .into_response()
}

/// `GET /api/auth/me` — returns the authenticated user.
///
/// For anonymous callers we return **401**, not `{authenticated:false}`,
/// because the existing SPA does `request<User>('/api/auth/me').catch(() => null)`
/// — it relies on the request rejecting, not on inspecting a JSON body, to
/// decide that there is no user.
pub async fn me(State(state): State<Arc<AppState>>) -> Response {
    let mut auth = match state.grpc_auth_client().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    let resp = match auth.who_am_i(WhoAmIRequest {}).await {
        Ok(r) => r.into_inner(),
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            return err(StatusCode::UNAUTHORIZED, s.message());
        }
        Err(s) => return err(StatusCode::BAD_GATEWAY, s.message()),
    };
    if !resp.authenticated {
        return err(StatusCode::UNAUTHORIZED, "not logged in");
    }
    let user = match resp.user.as_ref() {
        Some(u) => u,
        None => return err(StatusCode::BAD_GATEWAY, "no user in response"),
    };
    let dto = user_dto(user);
    Json(WhoAmIDto {
        username: dto.username.clone(),
        is_admin: dto.is_server_admin,
        authenticated: true,
        user: Some(dto),
        scopes: resp.scopes,
    })
    .into_response()
}

/// `GET /api/auth/initialized` — true once at least one user exists. Used
/// by the SPA to decide whether to render the setup wizard or the login
/// form. Anonymous client because a fresh-install browser obviously has no
/// session.
pub async fn is_initialized(State(state): State<Arc<AppState>>) -> Response {
    let mut auth = match state.grpc_auth_client_anonymous().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    match auth.is_server_initialized(IsServerInitializedRequest {}).await {
        Ok(r) => Json(InitializedDto {
            initialized: r.into_inner().initialized,
        })
        .into_response(),
        Err(s) => err(StatusCode::BAD_GATEWAY, s.message()),
    }
}

/// `POST /api/auth/bootstrap` — first-admin setup wizard. Forwards directly
/// to `AuthService::BootstrapAdmin`, which only accepts the call when the
/// users table is empty. Anonymous client because the first user has no
/// session.
pub async fn bootstrap_admin(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BootstrapBody>,
) -> Response {
    let mut auth = match state.grpc_auth_client_anonymous().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    let resp = match auth
        .bootstrap_admin(BootstrapAdminRequest {
            username: body.username,
            email: body.email,
            display_name: body.display_name,
            password: body.password,
        })
        .await
    {
        Ok(r) => r.into_inner(),
        Err(s) if s.code() == tonic::Code::FailedPrecondition => {
            return err(StatusCode::CONFLICT, s.message());
        }
        Err(s) if s.code() == tonic::Code::InvalidArgument => {
            return err(StatusCode::BAD_REQUEST, s.message());
        }
        Err(s) => return err(StatusCode::BAD_GATEWAY, s.message()),
    };
    let user = match resp.user {
        Some(u) => u,
        None => return err(StatusCode::BAD_GATEWAY, "no user in response"),
    };
    Json(serde_json::json!({"user": user_dto(&user)})).into_response()
}

/// `GET /api/auth/tokens` — list the current user's PATs (without plaintext).
pub async fn list_tokens(State(state): State<Arc<AppState>>) -> Response {
    let mut auth = match state.grpc_auth_client().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    match auth.list_personal_access_tokens(ListPatsRequest {}).await {
        Ok(r) => Json(r.into_inner().pats).into_response(),
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            err(StatusCode::UNAUTHORIZED, s.message())
        }
        Err(s) => err(StatusCode::BAD_GATEWAY, s.message()),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateTokenBody {
    pub name: String,
    pub scopes: Vec<String>,
    #[serde(default)]
    pub expires_at: i64,
}

/// `POST /api/auth/tokens` — mint a PAT for the current user.
pub async fn create_token(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateTokenBody>,
) -> Response {
    let mut auth = match state.grpc_auth_client().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    match auth
        .create_personal_access_token(CreatePatRequest {
            name: body.name,
            scopes: body.scopes,
            expires_at: body.expires_at,
        })
        .await
    {
        Ok(r) => Json(r.into_inner()).into_response(),
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            err(StatusCode::UNAUTHORIZED, s.message())
        }
        Err(s) if s.code() == tonic::Code::InvalidArgument => {
            err(StatusCode::BAD_REQUEST, s.message())
        }
        Err(s) => err(StatusCode::BAD_GATEWAY, s.message()),
    }
}

/// `DELETE /api/auth/tokens/:id` — revoke a PAT.
pub async fn delete_token(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Response {
    let mut auth = match state.grpc_auth_client().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    match auth.revoke_personal_access_token(RevokePatRequest { id }).await {
        Ok(r) => Json(r.into_inner()).into_response(),
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            err(StatusCode::UNAUTHORIZED, s.message())
        }
        Err(s) if s.code() == tonic::Code::PermissionDenied => {
            err(StatusCode::FORBIDDEN, s.message())
        }
        Err(s) => err(StatusCode::BAD_GATEWAY, s.message()),
    }
}

/// `GET /api/auth/sessions` — list the current user's active sessions.
pub async fn list_sessions(State(state): State<Arc<AppState>>) -> Response {
    let mut auth = match state.grpc_auth_client().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    match auth.list_my_sessions(ListSessionsRequest {}).await {
        Ok(r) => Json(r.into_inner().sessions).into_response(),
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            err(StatusCode::UNAUTHORIZED, s.message())
        }
        Err(s) => err(StatusCode::BAD_GATEWAY, s.message()),
    }
}

/// `DELETE /api/auth/sessions/:id` — revoke a session.
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Response {
    let mut auth = match state.grpc_auth_client().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    match auth.revoke_session(RevokeSessionRequest { id }).await {
        Ok(r) => Json(r.into_inner()).into_response(),
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            err(StatusCode::UNAUTHORIZED, s.message())
        }
        Err(s) if s.code() == tonic::Code::PermissionDenied => {
            err(StatusCode::FORBIDDEN, s.message())
        }
        Err(s) => err(StatusCode::BAD_GATEWAY, s.message()),
    }
}

/// `GET /api/auth/users` — server admin only.
pub async fn list_users(State(state): State<Arc<AppState>>) -> Response {
    let mut auth = match state.grpc_auth_client().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    match auth.list_users(ListUsersRequest {}).await {
        Ok(r) => Json(
            r.into_inner()
                .users
                .iter()
                .map(user_dto)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            err(StatusCode::UNAUTHORIZED, s.message())
        }
        Err(s) if s.code() == tonic::Code::PermissionDenied => {
            err(StatusCode::FORBIDDEN, s.message())
        }
        Err(s) => err(StatusCode::BAD_GATEWAY, s.message()),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateUserBody {
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub password: String,
    #[serde(default)]
    pub is_server_admin: bool,
}

/// `POST /api/auth/users` — server admin creates a new user.
pub async fn create_user(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateUserBody>,
) -> Response {
    let mut auth = match state.grpc_auth_client().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    match auth
        .create_user(CreateUserRequest {
            username: body.username,
            email: body.email,
            display_name: body.display_name,
            password: body.password,
            is_server_admin: body.is_server_admin,
        })
        .await
    {
        Ok(r) => {
            let user = r
                .into_inner()
                .user
                .map(|u| user_dto(&u))
                .unwrap_or_else(|| UserDto {
                    id: 0,
                    username: String::new(),
                    email: String::new(),
                    display_name: String::new(),
                    is_server_admin: false,
                });
            Json(serde_json::json!({ "user": user })).into_response()
        }
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            err(StatusCode::UNAUTHORIZED, s.message())
        }
        Err(s) if s.code() == tonic::Code::PermissionDenied => {
            err(StatusCode::FORBIDDEN, s.message())
        }
        Err(s) if s.code() == tonic::Code::InvalidArgument => {
            err(StatusCode::BAD_REQUEST, s.message())
        }
        Err(s) => err(StatusCode::BAD_GATEWAY, s.message()),
    }
}

/// `DELETE /api/auth/users/:id` — server admin deletes a user.
pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Response {
    let mut auth = match state.grpc_auth_client().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    match auth.delete_user(DeleteUserRequest { id }).await {
        Ok(r) => Json(r.into_inner()).into_response(),
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            err(StatusCode::UNAUTHORIZED, s.message())
        }
        Err(s) if s.code() == tonic::Code::PermissionDenied => {
            err(StatusCode::FORBIDDEN, s.message())
        }
        Err(s) => err(StatusCode::BAD_GATEWAY, s.message()),
    }
}

#[derive(Debug, Deserialize)]
pub struct GrantBody {
    pub user_id: i64,
    pub role: String,
}

/// `POST /api/auth/repos/:repo/members` — grant a role on a repo.
pub async fn grant_repo_role(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(repo): axum::extract::Path<String>,
    Json(body): Json<GrantBody>,
) -> Response {
    let mut auth = match state.grpc_auth_client().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    match auth
        .grant_repo_role(GrantRepoRoleRequest {
            repo,
            user_id: body.user_id,
            role: body.role,
        })
        .await
    {
        Ok(r) => Json(r.into_inner()).into_response(),
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            err(StatusCode::UNAUTHORIZED, s.message())
        }
        Err(s) if s.code() == tonic::Code::PermissionDenied => {
            err(StatusCode::FORBIDDEN, s.message())
        }
        Err(s) if s.code() == tonic::Code::InvalidArgument => {
            err(StatusCode::BAD_REQUEST, s.message())
        }
        Err(s) => err(StatusCode::BAD_GATEWAY, s.message()),
    }
}

/// `DELETE /api/auth/repos/:repo/members/:user_id` — revoke a role.
pub async fn revoke_repo_role(
    State(state): State<Arc<AppState>>,
    axum::extract::Path((repo, user_id)): axum::extract::Path<(String, i64)>,
) -> Response {
    let mut auth = match state.grpc_auth_client().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    match auth
        .revoke_repo_role(RevokeRepoRoleRequest { repo, user_id })
        .await
    {
        Ok(r) => Json(r.into_inner()).into_response(),
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            err(StatusCode::UNAUTHORIZED, s.message())
        }
        Err(s) if s.code() == tonic::Code::PermissionDenied => {
            err(StatusCode::FORBIDDEN, s.message())
        }
        Err(s) => err(StatusCode::BAD_GATEWAY, s.message()),
    }
}

/// `GET /api/auth/repos/:repo/members` — list repo members.
pub async fn list_repo_members(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(repo): axum::extract::Path<String>,
) -> Response {
    let mut auth = match state.grpc_auth_client().await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("forge-server: {e}")),
    };
    match auth.list_repo_members(ListRepoMembersRequest { repo }).await {
        Ok(r) => Json(r.into_inner().members).into_response(),
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            err(StatusCode::UNAUTHORIZED, s.message())
        }
        Err(s) if s.code() == tonic::Code::PermissionDenied => {
            err(StatusCode::FORBIDDEN, s.message())
        }
        Err(s) => err(StatusCode::BAD_GATEWAY, s.message()),
    }
}
