// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;

// ---------------------------------------------------------------------------
// JWT Claims
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,     // username
    is_admin: bool,
    exp: usize,      // expiry (unix timestamp)
    iat: usize,      // issued at
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
    pub is_admin: bool,
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

    // Create JWT.
    let now = chrono::Utc::now().timestamp() as usize;
    let ttl_secs = state.config.auth.token_ttl_hours as usize * 3600;
    let claims = Claims {
        sub: "admin".to_string(),
        is_admin: true,
        exp: now + ttl_secs,
        iat: now,
    };

    let token = match encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.config.auth.jwt_secret.as_bytes()),
    ) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("JWT encode error: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "internal error".to_string(),
                }),
            )
                .into_response();
        }
    };

    // Set cookie.
    let cookie_value = format!(
        "forge_token={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={ttl_secs}",
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
pub async fn logout() -> Response {
    let cookie_value =
        "forge_token=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".to_string();

    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie_value)],
        Json(serde_json::json!({"ok": true})),
    )
        .into_response()
}

/// GET /api/auth/me — returns info about the current user from the JWT.
pub async fn me(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
) -> Response {
    let token = match extract_token(&req) {
        Some(t) => t,
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

    match verify_token(&token, &state.config.auth.jwt_secret) {
        Ok(claims) => (
            StatusCode::OK,
            Json(MeResponse {
                username: claims.sub,
                is_admin: claims.is_admin,
            }),
        )
            .into_response(),
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "token expired or invalid".to_string(),
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
    let token = match extract_token(&req) {
        Some(t) => t,
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

    match verify_token(&token, &state.config.auth.jwt_secret) {
        Ok(_) => next.run(req).await,
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "token expired or invalid".to_string(),
            }),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the JWT from the `forge_token` cookie.
fn extract_token<B>(req: &Request<B>) -> Option<String> {
    let cookie_header = req.headers().get(header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("forge_token=") {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Verify and decode a JWT, returning the claims on success.
fn verify_token(token: &str, secret: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let validation = Validation::default();
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;
    Ok(token_data.claims)
}
