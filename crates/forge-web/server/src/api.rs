// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;

/// Characters to percent-encode for the quoted `filename` in Content-Disposition.
/// We stay conservative — anything outside ASCII alphanumerics plus a few
/// obviously-safe punctuation chars is encoded.
const DISPOSITION_FILENAME: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'\\')
    .add(b';')
    .add(b',')
    .add(b'(')
    .add(b')')
    .add(b'<')
    .add(b'>')
    .add(b'@')
    .add(b':')
    .add(b'?')
    .add(b'=')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'[')
    .add(b']')
    .add(b'%');

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Error responder. If the underlying error is a tonic Status (the common
/// case — almost every web handler proxies a gRPC call), translate the gRPC
/// code to the matching HTTP status so the SPA's interceptor can react
/// (401 → redirect to /login, 403 → show forbidden, etc.). Falls back to 500
/// for everything else.
pub(crate) fn internal_error_public(err: anyhow::Error) -> Response {
    internal_error(err)
}

fn internal_error(err: anyhow::Error) -> Response {
    // anyhow's downcast_ref works on the error chain.
    if let Some(status) = err.downcast_ref::<tonic::Status>() {
        return grpc_status_to_response(status);
    }
    // Sometimes the gRPC client wraps the Status inside another anyhow::Error
    // layer; walk the source chain just in case.
    let mut source: Option<&dyn std::error::Error> = err.source();
    while let Some(s) = source {
        if let Some(status) = s.downcast_ref::<tonic::Status>() {
            return grpc_status_to_response(status);
        }
        source = s.source();
    }
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": err.to_string()})),
    )
        .into_response()
}

fn grpc_status_to_response(status: &tonic::Status) -> Response {
    use tonic::Code;
    let code = status.code();
    let http = match code {
        Code::Unauthenticated => StatusCode::UNAUTHORIZED,
        Code::PermissionDenied => StatusCode::FORBIDDEN,
        Code::NotFound => StatusCode::NOT_FOUND,
        Code::InvalidArgument => StatusCode::BAD_REQUEST,
        Code::AlreadyExists => StatusCode::CONFLICT,
        Code::FailedPrecondition => StatusCode::CONFLICT,
        Code::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
        Code::Unimplemented => StatusCode::NOT_IMPLEMENTED,
        Code::DeadlineExceeded => StatusCode::GATEWAY_TIMEOUT,
        Code::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };

    // Only forward the message for codes whose messages are known to come from
    // our own handlers (validation errors, permission denials, etc). Internal /
    // Unknown / DataLoss / Unavailable often carry raw DB or network error
    // strings — those get a generic response and a server-side log entry.
    let is_safe = matches!(
        code,
        Code::Unauthenticated
            | Code::PermissionDenied
            | Code::NotFound
            | Code::InvalidArgument
            | Code::AlreadyExists
            | Code::FailedPrecondition
            | Code::ResourceExhausted
            | Code::DeadlineExceeded
            | Code::Unimplemented
    );
    let msg = if is_safe {
        status.message().to_string()
    } else {
        tracing::error!(
            code = ?code,
            upstream_msg = status.message(),
            "upstream gRPC error — masking from client"
        );
        "internal server error".to_string()
    };

    (http, Json(serde_json::json!({"error": msg}))).into_response()
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct RepoInfoJson {
    name: String,
    description: String,
    created_at: i64,
    branch_count: i32,
    default_branch: String,
    last_commit_message: String,
    last_commit_author: String,
    last_commit_time: i64,
    visibility: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateRepoBody {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRepoBody {
    pub new_name: Option<String>,
    pub description: Option<String>,
    /// "private" | "public" | None (no change)
    pub visibility: Option<String>,
    pub default_branch: Option<String>,
}

#[derive(Debug, Serialize)]
struct BranchInfo {
    name: String,
    head: String,
}

#[derive(Debug, Deserialize)]
pub struct CommitListQuery {
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

#[derive(Debug, Serialize)]
struct CommitJson {
    hash: String,
    message: String,
    author_name: String,
    author_email: String,
    timestamp: i64,
    parent_hashes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TreeEntryJson {
    name: String,
    kind: String,
    hash: String,
    size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    asset_class: Option<String>,
}

#[derive(Debug, Serialize)]
struct AssetMetadataJson {
    asset_class: String,
    engine_version: String,
    package_flags: Vec<String>,
    dependencies: Vec<String>,
}

#[derive(Debug, Serialize)]
struct FileContentJson {
    content: Option<String>,
    size: u64,
    is_binary: bool,
    hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    asset_metadata: Option<AssetMetadataJson>,
}

#[derive(Debug, Serialize)]
struct DiffEntryJson {
    path: String,
    change_type: String,
    old_size: u64,
    new_size: u64,
}

#[derive(Debug, Serialize)]
struct CommitDetailJson {
    commit: Option<CommitJson>,
    changes: Vec<DiffEntryJson>,
}

#[derive(Debug, Serialize)]
struct LockJson {
    path: String,
    owner: String,
    workspace_id: String,
    created_at: i64,
    reason: String,
}

#[derive(Debug, Deserialize)]
pub struct LockAcquireBody {
    pub path: String,
    pub owner: Option<String>,
    pub workspace_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LockReleaseQuery {
    pub owner: Option<String>,
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct LockListQuery {
    pub path_prefix: Option<String>,
    pub owner: Option<String>,
}

#[derive(Debug, Serialize)]
struct ServerInfoJson {
    version: String,
    uptime_secs: i64,
    total_objects: i64,
    total_size_bytes: i64,
    branches: Vec<String>,
    active_locks: i32,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/repos -- list all repositories.
pub async fn list_repos(State(state): State<Arc<AppState>>) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    match grpc.list_repos().await {
        Ok(resp) => {
            let repos: Vec<RepoInfoJson> = resp
                .repos
                .into_iter()
                .map(|r| RepoInfoJson {
                    name: r.name,
                    description: r.description,
                    created_at: r.created_at,
                    branch_count: r.branch_count,
                    default_branch: r.default_branch,
                    last_commit_message: r.last_commit_message,
                    last_commit_author: r.last_commit_author,
                    last_commit_time: r.last_commit_time,
                    visibility: r.visibility,
                })
                .collect();
            (StatusCode::OK, Json(repos)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

/// POST /api/repos -- create a repository.
pub async fn create_repo(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateRepoBody>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    let description = body.description.unwrap_or_default();
    match grpc.create_repo(&body.name, &description).await {
        Ok(resp) => {
            if resp.success {
                (
                    StatusCode::CREATED,
                    Json(serde_json::json!({"success": true})),
                )
                    .into_response()
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"success": false, "error": resp.error})),
                )
                    .into_response()
            }
        }
        Err(e) => internal_error(e),
    }
}

/// PUT /api/repos/:repo -- update repo (rename/description).
pub async fn update_repo(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
    Json(body): Json<UpdateRepoBody>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    let new_name = body.new_name.unwrap_or_default();
    let description = body.description.unwrap_or_default();
    let visibility = body.visibility.unwrap_or_default();
    let default_branch = body.default_branch.unwrap_or_default();

    match grpc.update_repo(&repo, &new_name, &description, &visibility, &default_branch).await {
        Ok(resp) => {
            if resp.success {
                (StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response()
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"success": false, "error": resp.error})),
                )
                    .into_response()
            }
        }
        Err(e) => internal_error(e),
    }
}

/// DELETE /api/repos/:repo -- delete a repo.
pub async fn delete_repo(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    match grpc.delete_repo(&repo).await {
        Ok(resp) => {
            if resp.success {
                (StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response()
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"success": false, "error": resp.error})),
                )
                    .into_response()
            }
        }
        Err(e) => internal_error(e),
    }
}

/// GET /api/repos/:repo/branches -- list branches for a repo.
pub async fn list_branches(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    match grpc.get_refs(&repo).await {
        Ok(resp) => {
            let branches: Vec<BranchInfo> = resp
                .refs
                .iter()
                .filter_map(|(name, hash_bytes)| {
                    let short = name.strip_prefix("refs/heads/")?;
                    Some(BranchInfo {
                        name: short.to_string(),
                        head: hex::encode(hash_bytes),
                    })
                })
                .collect();
            (StatusCode::OK, Json(branches)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

/// GET /api/repos/:repo/commits/:branch
pub async fn list_commits(
    State(state): State<Arc<AppState>>,
    Path((repo, branch)): Path<(String, String)>,
    Query(query): Query<CommitListQuery>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    let limit = query.limit.unwrap_or(50);
    let offset = query.offset.unwrap_or(0);

    match grpc.list_commits(&repo, &branch, limit, offset).await {
        Ok(resp) => {
            let commits: Vec<CommitJson> = resp
                .commits
                .into_iter()
                .map(|c| CommitJson {
                    hash: c.hash,
                    message: c.message,
                    author_name: c.author_name,
                    author_email: c.author_email,
                    timestamp: c.timestamp,
                    parent_hashes: c.parent_hashes,
                })
                .collect();
            let body = serde_json::json!({
                "commits": commits,
                "total": resp.total,
            });
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct TreeQuery {
    pub path: Option<String>,
}

/// GET /api/repos/:repo/tree/:branch?path=some/dir -- browse directory tree.
pub async fn get_tree(
    State(state): State<Arc<AppState>>,
    Path((repo, branch)): Path<(String, String)>,
    Query(query): Query<TreeQuery>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    let path = query.path.unwrap_or_default();

    let commit_hash = match resolve_branch(&grpc, &repo, &branch).await {
        Ok(h) => h,
        Err(e) => return internal_error(e),
    };

    match grpc.get_tree_entries(&repo, &commit_hash, &path).await {
        Ok(resp) => {
            let entries: Vec<TreeEntryJson> = resp
                .entries
                .into_iter()
                .map(|e| {
                    let asset_class = if e.asset_class.is_empty() { None } else { Some(e.asset_class) };
                    TreeEntryJson {
                        name: e.name,
                        kind: e.kind,
                        hash: e.hash,
                        size: e.size,
                        asset_class,
                    }
                })
                .collect();
            let body = serde_json::json!({
                "commit_hash": resp.commit_hash,
                "path": resp.path,
                "entries": entries,
            });
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct BlobQuery {
    pub path: String,
}

/// GET /api/repos/:repo/blob/:branch?path=some/file.txt -- get file content.
pub async fn get_blob(
    State(state): State<Arc<AppState>>,
    Path((repo, branch)): Path<(String, String)>,
    Query(query): Query<BlobQuery>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    let commit_hash = match resolve_branch(&grpc, &repo, &branch).await {
        Ok(h) => h,
        Err(e) => return internal_error(e),
    };

    match grpc.get_file_content(&repo, &commit_hash, &query.path).await {
        Ok(resp) => {
            let content = if resp.is_binary {
                None
            } else {
                Some(String::from_utf8_lossy(&resp.content).to_string())
            };
            let asset_metadata = resp.asset_metadata.map(|m| AssetMetadataJson {
                asset_class: m.asset_class,
                engine_version: m.engine_version,
                package_flags: m.package_flags,
                dependencies: m.dependencies,
            });
            let body = FileContentJson {
                content,
                size: resp.size,
                is_binary: resp.is_binary,
                hash: resp.hash,
                asset_metadata,
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

/// GET /api/repos/:repo/raw/:branch?path=file -- raw file download.
pub async fn get_raw(
    State(state): State<Arc<AppState>>,
    Path((repo, branch)): Path<(String, String)>,
    Query(query): Query<BlobQuery>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    let commit_hash = match resolve_branch(&grpc, &repo, &branch).await {
        Ok(h) => h,
        Err(e) => return internal_error(e),
    };

    match grpc.get_file_content(&repo, &commit_hash, &query.path).await {
        Ok(resp) => {
            // Basename of the requested path. Percent-encode it for both the
            // ASCII-fallback `filename=` and the RFC 5987 `filename*` form so
            // a commit that contains quotes, semicolons, or non-ASCII
            // characters cannot inject header content.
            let raw_name = query.path.rsplit('/').next().unwrap_or("file");
            let encoded: String =
                utf8_percent_encode(raw_name, DISPOSITION_FILENAME).collect();
            let content_type = if resp.is_binary {
                "application/octet-stream"
            } else {
                "text/plain; charset=utf-8"
            };
            let disposition = format!(
                "attachment; filename=\"{encoded}\"; filename*=UTF-8''{encoded}"
            );

            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, content_type.to_string()),
                    (header::CONTENT_DISPOSITION, disposition),
                    (header::CONTENT_LENGTH, resp.content.len().to_string()),
                ],
                resp.content,
            )
                .into_response()
        }
        Err(e) => internal_error(e),
    }
}

/// GET /api/repos/:repo/commit/:hash -- commit detail including changed files.
pub async fn get_commit(
    State(state): State<Arc<AppState>>,
    Path((repo, hash)): Path<(String, String)>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    match grpc.get_commit_detail(&repo, &hash).await {
        Ok(resp) => {
            let commit = resp.commit.map(|c| CommitJson {
                hash: c.hash,
                message: c.message,
                author_name: c.author_name,
                author_email: c.author_email,
                timestamp: c.timestamp,
                parent_hashes: c.parent_hashes,
            });
            let changes: Vec<DiffEntryJson> = resp
                .changes
                .into_iter()
                .map(|d| DiffEntryJson {
                    path: d.path,
                    change_type: d.change_type,
                    old_size: d.old_size,
                    new_size: d.new_size,
                })
                .collect();
            let body = CommitDetailJson { commit, changes };
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

/// GET /api/repos/:repo/locks -- list locks for a repo.
pub async fn list_locks(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
    Query(query): Query<LockListQuery>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    let prefix = query.path_prefix.unwrap_or_default();
    let owner = query.owner.unwrap_or_default();

    match grpc.list_locks(&repo, &prefix, &owner).await {
        Ok(resp) => {
            let locks: Vec<LockJson> = resp
                .locks
                .into_iter()
                .map(|l| LockJson {
                    path: l.path,
                    owner: l.owner,
                    workspace_id: l.workspace_id,
                    created_at: l.created_at,
                    reason: l.reason,
                })
                .collect();
            (StatusCode::OK, Json(locks)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

/// POST /api/repos/:repo/locks/acquire -- acquire a lock.
pub async fn acquire_lock(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
    Json(body): Json<LockAcquireBody>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    let owner = body.owner.unwrap_or_else(|| "web-admin".to_string());
    let workspace_id = body.workspace_id.unwrap_or_default();
    let reason = body.reason.unwrap_or_default();

    match grpc.acquire_lock(&repo, &body.path, &owner, &workspace_id, &reason).await {
        Ok(resp) => {
            let existing = resp.existing_lock.map(|l| LockJson {
                path: l.path,
                owner: l.owner,
                workspace_id: l.workspace_id,
                created_at: l.created_at,
                reason: l.reason,
            });
            let body = serde_json::json!({
                "granted": resp.granted,
                "existing_lock": existing,
            });
            let status = if resp.granted {
                StatusCode::OK
            } else {
                StatusCode::CONFLICT
            };
            (status, Json(body)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

/// DELETE /api/repos/:repo/locks/:path -- release a lock.
pub async fn release_lock(
    State(state): State<Arc<AppState>>,
    Path((repo, path)): Path<(String, String)>,
    Query(query): Query<LockReleaseQuery>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    let owner = query.owner.unwrap_or_else(|| "web-admin".to_string());
    let force = query.force.unwrap_or(false);

    match grpc.release_lock(&repo, &path, &owner, force).await {
        Ok(resp) => {
            if resp.success {
                (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"ok": false, "error": resp.error})),
                )
                    .into_response()
            }
        }
        Err(e) => internal_error(e),
    }
}

/// GET /api/server/info -- server statistics (admin only).
pub async fn server_info(State(state): State<Arc<AppState>>) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    match grpc.get_server_info().await {
        Ok(resp) => {
            let body = ServerInfoJson {
                version: resp.version,
                uptime_secs: resp.uptime_secs,
                total_objects: resp.total_objects,
                total_size_bytes: resp.total_size_bytes,
                branches: resp.repos,
                active_locks: resp.active_locks,
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// Issues
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct IssueQuery {
    pub status: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

#[derive(Debug, Serialize)]
struct IssueJson {
    id: i64,
    title: String,
    body: String,
    author: String,
    status: String,
    labels: Vec<String>,
    created_at: i64,
    updated_at: i64,
    comment_count: i32,
}

pub async fn list_issues(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
    Query(q): Query<IssueQuery>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.list_issues(&repo, q.status.as_deref().unwrap_or(""), q.limit.unwrap_or(50), q.offset.unwrap_or(0)).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    let issues: Vec<IssueJson> = resp.issues.into_iter().map(|i| IssueJson {
        id: i.id, title: i.title, body: i.body, author: i.author,
        status: i.status, labels: i.labels, created_at: i.created_at,
        updated_at: i.updated_at, comment_count: i.comment_count,
    }).collect();
    Json(serde_json::json!({
        "issues": issues,
        "total": resp.total,
        "open_count": resp.open_count,
        "closed_count": resp.closed_count,
    })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreateIssueBody {
    pub title: String,
    pub body: Option<String>,
    pub author: Option<String>,
    pub labels: Option<Vec<String>>,
}

pub async fn create_issue(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
    Json(body): Json<CreateIssueBody>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.create_issue(
        &repo, &body.title, body.body.as_deref().unwrap_or(""),
        body.author.as_deref().unwrap_or("web-user"),
        body.labels.unwrap_or_default(),
    ).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    Json(serde_json::json!({ "success": resp.success, "id": resp.id })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct UpdateIssueBody {
    pub title: Option<String>,
    pub body: Option<String>,
    pub status: Option<String>,
    pub labels: Option<Vec<String>>,
    pub assignee: Option<String>,
}

pub async fn update_issue(
    State(state): State<Arc<AppState>>,
    Path((_repo, id)): Path<(String, i64)>,
    Json(body): Json<UpdateIssueBody>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.update_issue(
        id, body.title.as_deref().unwrap_or(""), body.body.as_deref().unwrap_or(""),
        body.status.as_deref().unwrap_or(""), body.labels.unwrap_or_default(),
        body.assignee.as_deref().unwrap_or(""),
    ).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    Json(serde_json::json!({ "success": resp.success })).into_response()
}

// ---------------------------------------------------------------------------
// Pull Requests
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct PullRequestJson {
    id: i64,
    title: String,
    body: String,
    author: String,
    status: String,
    source_branch: String,
    target_branch: String,
    labels: Vec<String>,
    created_at: i64,
    updated_at: i64,
    comment_count: i32,
}

pub async fn list_pull_requests(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
    Query(q): Query<IssueQuery>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.list_pull_requests(&repo, q.status.as_deref().unwrap_or(""), q.limit.unwrap_or(50), q.offset.unwrap_or(0)).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    let prs: Vec<PullRequestJson> = resp.pull_requests.into_iter().map(|p| PullRequestJson {
        id: p.id, title: p.title, body: p.body, author: p.author,
        status: p.status, source_branch: p.source_branch, target_branch: p.target_branch,
        labels: p.labels, created_at: p.created_at, updated_at: p.updated_at,
        comment_count: p.comment_count,
    }).collect();
    Json(serde_json::json!({
        "pull_requests": prs,
        "total": resp.total,
        "open_count": resp.open_count,
        "closed_count": resp.closed_count,
    })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreatePrBody {
    pub title: String,
    pub body: Option<String>,
    pub author: Option<String>,
    pub source_branch: String,
    pub target_branch: Option<String>,
    pub labels: Option<Vec<String>>,
}

pub async fn create_pull_request(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
    Json(body): Json<CreatePrBody>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.create_pull_request(
        &repo, &body.title, body.body.as_deref().unwrap_or(""),
        body.author.as_deref().unwrap_or("web-user"),
        &body.source_branch, body.target_branch.as_deref().unwrap_or("main"),
        body.labels.unwrap_or_default(),
    ).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    Json(serde_json::json!({ "success": resp.success, "id": resp.id })).into_response()
}

pub async fn update_pull_request(
    State(state): State<Arc<AppState>>,
    Path((_repo, id)): Path<(String, i64)>,
    Json(body): Json<UpdateIssueBody>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.update_pull_request(
        id, body.title.as_deref().unwrap_or(""), body.body.as_deref().unwrap_or(""),
        body.status.as_deref().unwrap_or(""), body.labels.unwrap_or_default(),
        body.assignee.as_deref().unwrap_or(""),
    ).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    Json(serde_json::json!({ "success": resp.success })).into_response()
}

pub async fn merge_pull_request(
    State(state): State<Arc<AppState>>,
    Path((_repo, id)): Path<(String, i64)>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.merge_pull_request(id).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    Json(serde_json::json!({ "success": resp.success, "error": resp.error })).into_response()
}

pub async fn get_issue(
    State(state): State<Arc<AppState>>,
    Path((_repo, id)): Path<(String, i64)>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.get_issue(id).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    match resp.issue {
        Some(i) => Json(serde_json::json!({
            "id": i.id, "title": i.title, "body": i.body, "author": i.author,
            "status": i.status, "labels": i.labels, "assignee": i.assignee,
            "created_at": i.created_at, "updated_at": i.updated_at,
            "comment_count": i.comment_count,
        })).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"}))).into_response(),
    }
}

pub async fn get_pull_request(
    State(state): State<Arc<AppState>>,
    Path((_repo, id)): Path<(String, i64)>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.get_pull_request(id).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    match resp.pull_request {
        Some(p) => Json(serde_json::json!({
            "id": p.id, "title": p.title, "body": p.body, "author": p.author,
            "status": p.status, "source_branch": p.source_branch, "target_branch": p.target_branch,
            "labels": p.labels, "assignee": p.assignee,
            "created_at": p.created_at, "updated_at": p.updated_at,
            "comment_count": p.comment_count,
        })).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"}))).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Language statistics
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BranchQuery {
    pub branch: Option<String>,
}

#[derive(Debug, Serialize)]
struct LanguageStatJson {
    name: String,
    color: String,
    percentage: f64,
    bytes: u64,
    count: u64,
}

/// GET /api/repos/:repo/stats/languages -- language breakdown by file extension.
pub async fn language_stats(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
    Query(query): Query<BranchQuery>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    let branch = query.branch.as_deref().unwrap_or("main");
    let commit_hash = match resolve_branch(&grpc, &repo, branch).await {
        Ok(h) => h,
        Err(e) => return internal_error(e),
    };

    // Recursively walk the tree to accumulate bytes per extension.
    use std::collections::HashMap;

    struct ExtStats {
        bytes: u64,
        count: u64,
    }

    let mut ext_map: HashMap<String, ExtStats> = HashMap::new();
    let mut dirs_to_visit: Vec<String> = vec![String::new()];

    while let Some(dir_path) = dirs_to_visit.pop() {
        let resp = match grpc.get_tree_entries(&repo, &commit_hash, &dir_path).await {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in resp.entries {
            if entry.kind == "directory" || entry.kind == "dir" || entry.kind == "tree" {
                let child = if dir_path.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{}/{}", dir_path, entry.name)
                };
                dirs_to_visit.push(child);
            } else {
                // Extract extension.
                let ext = entry
                    .name
                    .rsplit('.')
                    .next()
                    .map(|e| format!(".{}", e.to_lowercase()))
                    .unwrap_or_default();
                if !ext.is_empty() && ext != format!(".{}", entry.name.to_lowercase()) {
                    let stats = ext_map.entry(ext).or_insert(ExtStats { bytes: 0, count: 0 });
                    stats.bytes += entry.size;
                    stats.count += 1;
                }
            }
        }
    }

    // Map extensions to language names and colors.
    fn ext_to_language(ext: &str) -> Option<(&'static str, &'static str)> {
        match ext {
            ".cpp" | ".h" | ".hpp" | ".c" => Some(("C++", "#f34b7d")),
            ".cs" => Some(("C#", "#178600")),
            ".py" => Some(("Python", "#3572A5")),
            ".rs" => Some(("Rust", "#dea584")),
            ".ts" | ".tsx" => Some(("TypeScript", "#3178c6")),
            ".js" | ".jsx" => Some(("JavaScript", "#f1e05a")),
            ".uasset" | ".uexp" | ".ubulk" => Some(("Unreal Assets", "#2f3640")),
            ".umap" => Some(("Unreal Maps", "#6c3483")),
            ".ini" | ".toml" | ".json" | ".yaml" | ".yml" => Some(("Config", "#8B8B8B")),
            ".md" | ".txt" | ".rst" => Some(("Documentation", "#083fa1")),
            ".glsl" | ".hlsl" | ".ush" | ".usf" => Some(("Shaders", "#5e97d0")),
            _ => None,
        }
    }

    // Aggregate by language.
    let mut lang_map: HashMap<&str, (u64, u64, &str)> = HashMap::new(); // name -> (bytes, count, color)
    let mut other_bytes: u64 = 0;
    let mut other_count: u64 = 0;

    for (ext, stats) in &ext_map {
        if let Some((name, color)) = ext_to_language(ext) {
            let entry = lang_map.entry(name).or_insert((0, 0, color));
            entry.0 += stats.bytes;
            entry.1 += stats.count;
        } else {
            other_bytes += stats.bytes;
            other_count += stats.count;
        }
    }

    if other_bytes > 0 {
        let entry = lang_map.entry("Other").or_insert((0, 0, "#cccccc"));
        entry.0 += other_bytes;
        entry.1 += other_count;
    }

    let total_bytes: u64 = lang_map.values().map(|(b, _, _)| *b).sum();

    let mut languages: Vec<LanguageStatJson> = lang_map
        .into_iter()
        .map(|(name, (bytes, count, color))| {
            let percentage = if total_bytes > 0 {
                (bytes as f64 / total_bytes as f64) * 100.0
            } else {
                0.0
            };
            // Round to one decimal.
            let percentage = (percentage * 10.0).round() / 10.0;
            LanguageStatJson {
                name: name.to_string(),
                color: color.to_string(),
                percentage,
                bytes,
                count,
            }
        })
        .collect();

    languages.sort_by(|a, b| b.percentage.partial_cmp(&a.percentage).unwrap_or(std::cmp::Ordering::Equal));

    (StatusCode::OK, Json(serde_json::json!({ "languages": languages }))).into_response()
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CommentQuery {
    pub issue_id: Option<i64>,
    pub kind: Option<String>,
}

pub async fn list_comments(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
    Query(q): Query<CommentQuery>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.list_comments(&repo, q.issue_id.unwrap_or(0), q.kind.as_deref().unwrap_or("issue")).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    Json(resp.comments).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreateCommentBody {
    pub issue_id: i64,
    pub kind: Option<String>,
    pub body: String,
}

pub async fn create_comment(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
    Json(body): Json<CreateCommentBody>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.create_comment(&repo, body.issue_id, body.kind.as_deref().unwrap_or("issue"), "web-user", &body.body).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    Json(serde_json::json!({ "success": resp.success, "id": resp.id })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct UpdateCommentBody {
    pub body: String,
}

pub async fn update_comment(
    State(state): State<Arc<AppState>>,
    Path((_repo, id)): Path<(String, i64)>,
    Json(body): Json<UpdateCommentBody>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.update_comment(id, &body.body).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    Json(serde_json::json!({ "success": resp.success })).into_response()
}

pub async fn delete_comment(
    State(state): State<Arc<AppState>>,
    Path((_repo, id)): Path<(String, i64)>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    let resp = match grpc.delete_comment(id).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    Json(serde_json::json!({ "success": resp.success })).into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve a branch name to its HEAD commit hash via GetRefs.
///
/// "Not found" cases (empty repo, missing commit, missing branch) are
/// returned as `tonic::Status::not_found` so the smart `internal_error`
/// mapper translates them to HTTP 404 — letting the SPA distinguish
/// "this repo has no commits yet, show quickstart" from "the server is
/// broken, show a flash error".
async fn resolve_branch(
    grpc: &crate::grpc_client::ForgeGrpcClient,
    repo: &str,
    branch: &str,
) -> anyhow::Result<String> {
    // If it's a full 64-char hex hash, use directly.
    if branch.len() == 64 && branch.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(branch.to_string());
    }

    // If it looks like a short commit hash (hex >= 12 chars), resolve via refs and commit walk.
    if branch.len() >= 12 && branch.chars().all(|c| c.is_ascii_hexdigit()) {
        let refs_resp = grpc.get_refs(repo).await?;
        // Check if any ref tip matches the prefix.
        for hash_bytes in refs_resp.refs.values() {
            let full = hex::encode(hash_bytes);
            if full.starts_with(branch) {
                return Ok(full);
            }
        }
        // Walk recent commits from each branch to find the full hash.
        for hash_bytes in refs_resp.refs.values() {
            let tip = hex::encode(hash_bytes);
            if let Ok(commits) = grpc.list_commits(repo, &tip, 1, 200).await {
                for c in &commits.commits {
                    if c.hash.starts_with(branch) {
                        return Ok(c.hash.clone());
                    }
                }
            }
        }
        return Err(
            tonic::Status::not_found(format!("commit '{branch}' not found")).into(),
        );
    }

    let refs_resp = grpc.get_refs(repo).await?;
    let ref_name_candidates = [
        branch.to_string(),
        format!("refs/heads/{branch}"),
    ];

    for candidate in &ref_name_candidates {
        if let Some(hash_bytes) = refs_resp.refs.get(candidate) {
            return Ok(hex::encode(hash_bytes));
        }
    }

    Err(tonic::Status::not_found(format!("branch '{branch}' not found")).into())
}
