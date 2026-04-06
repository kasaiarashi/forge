// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::AppState;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A logged-in session.
#[derive(Debug, Clone)]
pub struct Session {
    pub session_id: String,
    pub username: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Thread-safe session store.
pub type SessionStore = Arc<RwLock<HashMap<String, Session>>>;

pub fn new_session_store() -> SessionStore {
    Arc::new(RwLock::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Request / response DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub ok: bool,
    pub username: String,
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub username: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/auth/login
pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> Response {
    // Only the "admin" user is supported.
    if body.username != "admin" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "invalid credentials".to_string(),
            }),
        )
            .into_response();
    }

    // Verify password against the bcrypt hash stored in config.
    let hash = &state.config.auth.admin_password_hash;
    if hash.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorBody {
                error: "admin password not configured — run `forge-web init` first".to_string(),
            }),
        )
            .into_response();
    }

    match bcrypt::verify(&body.password, hash) {
        Ok(true) => { /* valid */ }
        Ok(false) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorBody {
                    error: "invalid credentials".to_string(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("bcrypt verify error: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "internal error".to_string(),
                }),
            )
                .into_response();
        }
    }

    // Create session.
    let session_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let ttl = TimeDelta::hours(state.config.auth.session_ttl_hours as i64);
    let session = Session {
        session_id: session_id.clone(),
        username: "admin".to_string(),
        created_at: now,
        expires_at: now + ttl,
    };

    state.sessions.write().await.insert(session_id.clone(), session);

    // Set cookie.
    let cookie_value = format!(
        "forge_session={session_id}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        ttl.num_seconds()
    );

    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie_value)],
        Json(LoginResponse {
            ok: true,
            username: "admin".to_string(),
        }),
    )
        .into_response()
}

/// POST /api/auth/logout
pub async fn logout(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
) -> Response {
    if let Some(session_id) = extract_session_id(&req) {
        state.sessions.write().await.remove(&session_id);
    }

    let cookie_value =
        "forge_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".to_string();

    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie_value)],
        Json(serde_json::json!({"ok": true})),
    )
        .into_response()
}

/// GET /api/auth/me — returns info about the current session.
pub async fn me(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
) -> Response {
    let session_id = match extract_session_id(&req) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorBody {
                    error: "not authenticated".to_string(),
                }),
            )
                .into_response();
        }
    };

    let sessions = state.sessions.read().await;
    match sessions.get(&session_id) {
        Some(s) if s.expires_at > Utc::now() => (
            StatusCode::OK,
            Json(MeResponse {
                username: s.username.clone(),
                created_at: s.created_at.to_rfc3339(),
                expires_at: s.expires_at.to_rfc3339(),
            }),
        )
            .into_response(),
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "session expired or invalid".to_string(),
            }),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

/// Middleware that rejects unauthenticated requests.
/// Attach this to route groups that require a logged-in user.
pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let session_id = match extract_session_id(&req) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorBody {
                    error: "authentication required".to_string(),
                }),
            )
                .into_response();
        }
    };

    let sessions = state.sessions.read().await;
    match sessions.get(&session_id) {
        Some(s) if s.expires_at > Utc::now() => {
            drop(sessions);
            next.run(req).await
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "session expired or invalid".to_string(),
            }),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the session ID from the `Cookie` header.
fn extract_session_id<B>(req: &Request<B>) -> Option<String> {
    let cookie_header = req.headers().get(header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("forge_session=") {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}
