// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! gRPC `AuthService` server implementation.
//!
//! This is the only place identity / sessions / PATs / ACLs are exposed over
//! the wire. Both the CLI and the web UI consume the same methods — the web
//! UI just stores the returned session token in an HttpOnly cookie instead
//! of `~/.forge/credentials`.
//!
//! Authentication for *each* method comes from the bearer token interceptor
//! in [`crate::auth::interceptor`]. Authorization (admin gating, "is this
//! caller acting on themself?") happens at the top of each handler.

use std::sync::Arc;
use tonic::{Request, Response, Status};

use forge_proto::forge::auth_service_server::AuthService;
use forge_proto::forge::*;

use crate::auth::authorize;
use crate::auth::interceptor::caller_of;
use crate::auth::store::{NewUser, RepoRole, UserStore};
use crate::auth::tokens::{self, Scope};

/// Log then mask — see the twin helper in services/grpc.rs.
fn internal_err<E: std::fmt::Display>(label: &'static str, err: E) -> Status {
    tracing::error!(op = label, error = %err, "internal error");
    Status::internal("internal server error")
}

const SESSION_TTL_SECONDS: i64 = 24 * 60 * 60; // 24h

pub struct ForgeAuthService {
    pub store: Arc<dyn UserStore>,
    /// One-time bootstrap token generated at first start. Required on
    /// `BootstrapAdmin` until the first user exists, then consumed (token
    /// file deleted).
    pub bootstrap_token: Option<String>,
    /// Path to the bootstrap token file so we can delete it after the first
    /// admin is created.
    pub bootstrap_token_path: std::path::PathBuf,
}

fn user_to_proto(u: &crate::auth::store::User) -> UserInfo {
    UserInfo {
        id: u.id,
        username: u.username.clone(),
        email: u.email.clone(),
        display_name: u.display_name.clone(),
        is_server_admin: u.is_server_admin,
        created_at: u.created_at,
        last_login_at: u.last_login_at.unwrap_or(0),
    }
}

fn session_to_proto(s: &crate::auth::store::Session) -> SessionInfo {
    SessionInfo {
        id: s.id,
        user_id: s.user_id,
        created_at: s.created_at,
        last_used_at: s.last_used_at,
        expires_at: s.expires_at,
        user_agent: s.user_agent.clone().unwrap_or_default(),
        ip: s.ip.clone().unwrap_or_default(),
    }
}

fn pat_to_proto(p: &crate::auth::store::PersonalAccessToken) -> PatInfo {
    PatInfo {
        id: p.id,
        name: p.name.clone(),
        user_id: p.user_id,
        scopes: p.scopes.iter().map(|s| s.as_str().to_string()).collect(),
        created_at: p.created_at,
        last_used_at: p.last_used_at.unwrap_or(0),
        expires_at: p.expires_at.unwrap_or(0),
    }
}

fn parse_scopes_proto(raw: &[String]) -> Result<Vec<Scope>, Status> {
    raw.iter()
        .map(|s| Scope::parse(s).map_err(|e| Status::invalid_argument(e.to_string())))
        .collect()
}

#[tonic::async_trait]
impl AuthService for ForgeAuthService {
    // ── Identity ────────────────────────────────────────────────────────────

    async fn login(
        &self,
        request: Request<LoginRequest>,
    ) -> Result<Response<LoginResponse>, Status> {
        let req = request.into_inner();
        if req.username.is_empty() || req.password.is_empty() {
            return Err(Status::invalid_argument("username and password required"));
        }
        let user = self
            .store
            .verify_password(&req.username, &req.password)
            .map_err(|e| internal_err("auth", e))?
            .ok_or_else(|| Status::unauthenticated("invalid username or password"))?;

        let token = self
            .store
            .create_session(
                user.id,
                SESSION_TTL_SECONDS,
                non_empty(&req.user_agent),
                non_empty(&req.ip),
            )
            .map_err(|e| internal_err("create session", e))?;

        Ok(Response::new(LoginResponse {
            session_token: token.plaintext,
            user: Some(user_to_proto(&user)),
            expires_at: token.session.expires_at,
        }))
    }

    async fn logout(
        &self,
        request: Request<LogoutRequest>,
    ) -> Result<Response<LogoutResponse>, Status> {
        // The interceptor already validated the bearer token; we look it up
        // again here to find the session row and revoke it. The CLI variant
        // (PAT) is a no-op because PATs are revoked through the explicit
        // RevokePersonalAccessToken API.
        let raw = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
            .map(str::to_string);

        if let Some(token) = raw {
            if token.starts_with(crate::auth::tokens::SESSION_PREFIX) {
                if let Some((session, _user)) = self
                    .store
                    .find_session_by_plaintext(&token)
                    .map_err(|e| internal_err("session lookup", e))?
                {
                    self.store
                        .revoke_session(session.id)
                        .map_err(|e| internal_err("revoke session", e))?;
                }
            }
        }
        Ok(Response::new(LogoutResponse { success: true }))
    }

    async fn who_am_i(
        &self,
        request: Request<WhoAmIRequest>,
    ) -> Result<Response<WhoAmIResponse>, Status> {
        let caller = caller_of(&request);
        match caller {
            crate::auth::Caller::Anonymous => Ok(Response::new(WhoAmIResponse {
                authenticated: false,
                user: None,
                scopes: vec![],
            })),
            crate::auth::Caller::Authenticated(a) => {
                let user = self
                    .store
                    .find_user_by_id(a.user_id)
                    .map_err(|e| internal_err("lookup", e))?
                    .ok_or_else(|| Status::not_found("user no longer exists"))?;
                let scopes = match a.credential {
                    crate::auth::caller::CredentialKind::Session => vec![
                        // Sessions are unscoped — report the full set so the
                        // CLI/web know what's allowed.
                        Scope::RepoRead.as_str().to_string(),
                        Scope::RepoWrite.as_str().to_string(),
                        Scope::RepoAdmin.as_str().to_string(),
                        Scope::UserAdmin.as_str().to_string(),
                    ],
                    crate::auth::caller::CredentialKind::PersonalAccessToken => {
                        a.scopes.iter().map(|s| s.as_str().to_string()).collect()
                    }
                };
                Ok(Response::new(WhoAmIResponse {
                    authenticated: true,
                    user: Some(user_to_proto(&user)),
                    scopes,
                }))
            }
        }
    }

    // ── PATs ────────────────────────────────────────────────────────────────

    async fn create_personal_access_token(
        &self,
        request: Request<CreatePatRequest>,
    ) -> Result<Response<CreatePatResponse>, Status> {
        let caller = caller_of(&request);
        let auth = authorize::require_authenticated(&caller)?;
        let req = request.into_inner();
        let scopes = parse_scopes_proto(&req.scopes)?;
        tokens::validate_scopes(&scopes).map_err(|e| Status::invalid_argument(e.to_string()))?;
        let expires = if req.expires_at == 0 {
            None
        } else {
            Some(req.expires_at)
        };
        let (pat, plaintext) = self
            .store
            .create_pat(auth.user_id, &req.name, &scopes, expires)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        Ok(Response::new(CreatePatResponse {
            plaintext_token: plaintext.plaintext,
            pat: Some(pat_to_proto(&pat)),
        }))
    }

    async fn list_personal_access_tokens(
        &self,
        request: Request<ListPatsRequest>,
    ) -> Result<Response<ListPatsResponse>, Status> {
        let caller = caller_of(&request);
        let auth = authorize::require_authenticated(&caller)?;
        let pats = self
            .store
            .list_pats_for_user(auth.user_id)
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(ListPatsResponse {
            pats: pats.iter().map(pat_to_proto).collect(),
        }))
    }

    async fn revoke_personal_access_token(
        &self,
        request: Request<RevokePatRequest>,
    ) -> Result<Response<RevokePatResponse>, Status> {
        let caller = caller_of(&request);
        let auth = authorize::require_authenticated(&caller)?;
        let req = request.into_inner();
        // Make sure the PAT belongs to the caller (or caller is server admin).
        let pats = self
            .store
            .list_pats_for_user(auth.user_id)
            .map_err(|e| internal_err("grpc", e))?;
        let owns = pats.iter().any(|p| p.id == req.id);
        if !owns && !auth.is_server_admin {
            return Err(Status::permission_denied("cannot revoke another user's token"));
        }
        let removed = self
            .store
            .revoke_pat(req.id)
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(RevokePatResponse { success: removed }))
    }

    // ── Sessions ────────────────────────────────────────────────────────────

    async fn list_my_sessions(
        &self,
        request: Request<ListSessionsRequest>,
    ) -> Result<Response<ListSessionsResponse>, Status> {
        let caller = caller_of(&request);
        let auth = authorize::require_authenticated(&caller)?;
        let sessions = self
            .store
            .list_sessions_for_user(auth.user_id)
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(ListSessionsResponse {
            sessions: sessions.iter().map(session_to_proto).collect(),
        }))
    }

    async fn revoke_session(
        &self,
        request: Request<RevokeSessionRequest>,
    ) -> Result<Response<RevokeSessionResponse>, Status> {
        let caller = caller_of(&request);
        let auth = authorize::require_authenticated(&caller)?;
        let req = request.into_inner();
        // Confirm ownership.
        let sessions = self
            .store
            .list_sessions_for_user(auth.user_id)
            .map_err(|e| internal_err("grpc", e))?;
        let owns = sessions.iter().any(|s| s.id == req.id);
        if !owns && !auth.is_server_admin {
            return Err(Status::permission_denied(
                "cannot revoke another user's session",
            ));
        }
        let removed = self
            .store
            .revoke_session(req.id)
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(RevokeSessionResponse { success: removed }))
    }

    // ── User admin ──────────────────────────────────────────────────────────

    async fn create_user(
        &self,
        request: Request<CreateUserRequest>,
    ) -> Result<Response<CreateUserResponse>, Status> {
        let caller = caller_of(&request);
        authorize::require_server_admin(&caller)?;
        let req = request.into_inner();
        let user = self
            .store
            .create_user(NewUser {
                username: req.username,
                email: req.email,
                display_name: req.display_name,
                password: req.password,
                is_server_admin: req.is_server_admin,
            })
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        Ok(Response::new(CreateUserResponse {
            user: Some(user_to_proto(&user)),
        }))
    }

    async fn list_users(
        &self,
        request: Request<ListUsersRequest>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        let caller = caller_of(&request);
        authorize::require_server_admin(&caller)?;
        let users = self
            .store
            .list_users()
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(ListUsersResponse {
            users: users.iter().map(user_to_proto).collect(),
        }))
    }

    async fn delete_user(
        &self,
        request: Request<DeleteUserRequest>,
    ) -> Result<Response<DeleteUserResponse>, Status> {
        let caller = caller_of(&request);
        authorize::require_server_admin(&caller)?;
        let req = request.into_inner();
        let removed = self
            .store
            .delete_user(req.id)
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(DeleteUserResponse { success: removed }))
    }

    // ── Repo ACLs ───────────────────────────────────────────────────────────

    async fn grant_repo_role(
        &self,
        request: Request<GrantRepoRoleRequest>,
    ) -> Result<Response<GrantRepoRoleResponse>, Status> {
        let caller = caller_of(&request);
        let granted_by = caller.user_id(); // capture before request is consumed
        let req = request.into_inner();
        authorize::require_repo_admin(&caller, &self.store, &req.repo)?;
        let role = RepoRole::parse(&req.role)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        self.store
            .set_repo_role(&req.repo, req.user_id, role, granted_by)
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(GrantRepoRoleResponse { success: true }))
    }

    async fn revoke_repo_role(
        &self,
        request: Request<RevokeRepoRoleRequest>,
    ) -> Result<Response<RevokeRepoRoleResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        authorize::require_repo_admin(&caller, &self.store, &req.repo)?;
        let removed = self
            .store
            .revoke_repo_role(&req.repo, req.user_id)
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(RevokeRepoRoleResponse { success: removed }))
    }

    async fn list_repo_members(
        &self,
        request: Request<ListRepoMembersRequest>,
    ) -> Result<Response<ListRepoMembersResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        authorize::require_repo_read(&caller, &self.store, &req.repo, false)?;
        let members = self
            .store
            .list_repo_members(&req.repo)
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(ListRepoMembersResponse {
            members: members
                .iter()
                .map(|(u, r)| RepoMember {
                    user: Some(user_to_proto(u)),
                    role: r.as_str().to_string(),
                })
                .collect(),
        }))
    }

    // ── Bootstrap ───────────────────────────────────────────────────────────

    async fn is_server_initialized(
        &self,
        _request: Request<IsServerInitializedRequest>,
    ) -> Result<Response<IsServerInitializedResponse>, Status> {
        let count = self
            .store
            .count_users()
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(IsServerInitializedResponse {
            initialized: count > 0,
        }))
    }

    async fn bootstrap_admin(
        &self,
        request: Request<BootstrapAdminRequest>,
    ) -> Result<Response<BootstrapAdminResponse>, Status> {
        // Hard guard: only allowed when the users table is empty.
        let count = self
            .store
            .count_users()
            .map_err(|e| {
                tracing::error!(error = %e, "count_users failed during bootstrap");
                Status::internal("internal server error")
            })?;
        if count > 0 {
            return Err(Status::failed_precondition(
                "server is already initialized — use Login to obtain a session",
            ));
        }

        // Bootstrap-token check: the server prints a one-time token on
        // first start and writes it to <base>/.bootstrap_token. Require it
        // so that a publicly-reachable fresh install cannot be hijacked by
        // whoever races to the bootstrap endpoint first.
        let req = request.into_inner();
        match self.bootstrap_token.as_deref() {
            Some(expected) => {
                if !constant_time_eq(expected.as_bytes(), req.bootstrap_token.as_bytes()) {
                    tracing::warn!("bootstrap_admin denied: missing or invalid bootstrap token");
                    return Err(Status::permission_denied(
                        "missing or invalid bootstrap token — see forge-server logs \
                         or the file <base_path>/.bootstrap_token",
                    ));
                }
            }
            None => {
                // No token configured on the server — refuse rather than
                // silently allowing an uncontrolled bootstrap.
                return Err(Status::failed_precondition(
                    "bootstrap token not initialized on the server",
                ));
            }
        }

        let user = self
            .store
            .create_user(NewUser {
                username: req.username,
                email: req.email,
                display_name: req.display_name,
                password: req.password,
                is_server_admin: true,
            })
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        // Consume the token so it can't be reused. We do this AFTER the
        // user is successfully created so a transient failure (duplicate
        // username, invalid password policy) doesn't lock the operator out.
        if let Err(e) = std::fs::remove_file(&self.bootstrap_token_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(
                    error = %e,
                    path = ?self.bootstrap_token_path,
                    "failed to delete bootstrap token file after successful bootstrap"
                );
            }
        }

        Ok(Response::new(BootstrapAdminResponse {
            user: Some(user_to_proto(&user)),
        }))
    }
}

/// Constant-time byte comparison. Returns false on length mismatch without
/// short-circuiting.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}

fn non_empty(s: &str) -> Option<&str> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
