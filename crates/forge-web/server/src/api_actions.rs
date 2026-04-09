// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! REST API handlers for Actions (workflows, runs, artifacts, releases).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;

/// Reuses the smart error mapper from `api.rs` so gRPC Unauthenticated /
/// PermissionDenied / NotFound surface as the right HTTP status, not 500.
fn internal_error(err: anyhow::Error) -> Response {
    crate::api::internal_error_public(err)
}

// ── DTOs ──

#[derive(Serialize)]
struct WorkflowJson {
    id: i64,
    name: String,
    yaml: String,
    enabled: bool,
    created_at: i64,
    updated_at: i64,
}

#[derive(Serialize)]
struct RunJson {
    id: i64,
    workflow_id: i64,
    workflow_name: String,
    trigger: String,
    trigger_ref: String,
    commit_hash: String,
    status: String,
    started_at: i64,
    finished_at: i64,
    created_at: i64,
    triggered_by: String,
}

#[derive(Serialize)]
struct StepJson {
    id: i64,
    job_name: String,
    step_index: i32,
    name: String,
    status: String,
    exit_code: i32,
    log: String,
    started_at: i64,
    finished_at: i64,
}

#[derive(Serialize)]
struct ArtifactJson {
    id: i64,
    run_id: i64,
    name: String,
    size_bytes: i64,
    created_at: i64,
}

#[derive(Serialize)]
struct ReleaseJson {
    id: i64,
    tag: String,
    name: String,
    run_id: i64,
    created_at: i64,
    artifacts: Vec<ArtifactJson>,
}

#[derive(Deserialize)]
pub struct CreateWorkflowBody {
    name: String,
    yaml: String,
}

#[derive(Deserialize)]
pub struct UpdateWorkflowBody {
    #[serde(default)]
    name: String,
    #[serde(default)]
    yaml: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool { true }

#[derive(Deserialize)]
pub struct TriggerBody {
    #[serde(default = "default_ref")]
    ref_name: String,
    #[serde(default)]
    triggered_by: String,
}

fn default_ref() -> String { "refs/heads/main".into() }

#[derive(Deserialize)]
pub struct RunsQuery {
    #[serde(default)]
    workflow_id: i64,
    #[serde(default = "default_limit")]
    limit: i32,
    #[serde(default)]
    offset: i32,
}

fn default_limit() -> i32 { 50 }

// ── Handlers ──

pub async fn list_workflows(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    match grpc.list_workflows(&repo).await {
        Ok(resp) => {
            let workflows: Vec<WorkflowJson> = resp.workflows.into_iter().map(|w| WorkflowJson {
                id: w.id, name: w.name, yaml: w.yaml, enabled: w.enabled,
                created_at: w.created_at, updated_at: w.updated_at,
            }).collect();
            (StatusCode::OK, Json(workflows)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

pub async fn create_workflow(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
    Json(body): Json<CreateWorkflowBody>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    match grpc.create_workflow(&repo, &body.name, &body.yaml).await {
        Ok(resp) => {
            if resp.success {
                (StatusCode::CREATED, Json(serde_json::json!({"success": true, "id": resp.id}))).into_response()
            } else {
                (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": resp.error}))).into_response()
            }
        }
        Err(e) => internal_error(e),
    }
}

pub async fn update_workflow(
    State(state): State<Arc<AppState>>,
    Path((_repo, id)): Path<(String, i64)>,
    Json(body): Json<UpdateWorkflowBody>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    match grpc.update_workflow(id, &body.name, &body.yaml, body.enabled).await {
        Ok(resp) => {
            if resp.success {
                (StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response()
            } else {
                (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": resp.error}))).into_response()
            }
        }
        Err(e) => internal_error(e),
    }
}

pub async fn delete_workflow(
    State(state): State<Arc<AppState>>,
    Path((_repo, id)): Path<(String, i64)>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    match grpc.delete_workflow(id).await {
        Ok(resp) => {
            if resp.success {
                (StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response()
            } else {
                (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": resp.error}))).into_response()
            }
        }
        Err(e) => internal_error(e),
    }
}

pub async fn trigger_workflow(
    State(state): State<Arc<AppState>>,
    Path((_repo, id)): Path<(String, i64)>,
    Json(body): Json<TriggerBody>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    match grpc.trigger_workflow(id, &body.ref_name, &body.triggered_by).await {
        Ok(resp) => {
            if resp.success {
                (StatusCode::CREATED, Json(serde_json::json!({"success": true, "run_id": resp.run_id}))).into_response()
            } else {
                (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": resp.error}))).into_response()
            }
        }
        Err(e) => internal_error(e),
    }
}

pub async fn list_runs(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
    Query(query): Query<RunsQuery>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    match grpc.list_workflow_runs(&repo, query.workflow_id, query.limit, query.offset).await {
        Ok(resp) => {
            let runs: Vec<RunJson> = resp.runs.into_iter().map(|r| RunJson {
                id: r.id, workflow_id: r.workflow_id, workflow_name: r.workflow_name,
                trigger: r.trigger, trigger_ref: r.trigger_ref, commit_hash: r.commit_hash,
                status: r.status, started_at: r.started_at, finished_at: r.finished_at,
                created_at: r.created_at, triggered_by: r.triggered_by,
            }).collect();
            (StatusCode::OK, Json(serde_json::json!({"runs": runs, "total": resp.total}))).into_response()
        }
        Err(e) => internal_error(e),
    }
}

pub async fn get_run(
    State(state): State<Arc<AppState>>,
    Path((_repo, run_id)): Path<(String, i64)>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    match grpc.get_workflow_run(run_id).await {
        Ok(resp) => {
            let run = resp.run.map(|r| RunJson {
                id: r.id, workflow_id: r.workflow_id, workflow_name: r.workflow_name,
                trigger: r.trigger, trigger_ref: r.trigger_ref, commit_hash: r.commit_hash,
                status: r.status, started_at: r.started_at, finished_at: r.finished_at,
                created_at: r.created_at, triggered_by: r.triggered_by,
            });
            let steps: Vec<StepJson> = resp.steps.into_iter().map(|s| StepJson {
                id: s.id, job_name: s.job_name, step_index: s.step_index,
                name: s.name, status: s.status, exit_code: s.exit_code,
                log: s.log, started_at: s.started_at, finished_at: s.finished_at,
            }).collect();
            let artifacts: Vec<ArtifactJson> = resp.artifacts.into_iter().map(|a| ArtifactJson {
                id: a.id, run_id: a.run_id, name: a.name,
                size_bytes: a.size_bytes, created_at: a.created_at,
            }).collect();
            (StatusCode::OK, Json(serde_json::json!({
                "run": run, "steps": steps, "artifacts": artifacts,
            }))).into_response()
        }
        Err(e) => internal_error(e),
    }
}

pub async fn cancel_run(
    State(state): State<Arc<AppState>>,
    Path((_repo, run_id)): Path<(String, i64)>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    match grpc.cancel_workflow_run(run_id).await {
        Ok(resp) => {
            if resp.success {
                (StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response()
            } else {
                (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": resp.error}))).into_response()
            }
        }
        Err(e) => internal_error(e),
    }
}

pub async fn list_artifacts(
    State(state): State<Arc<AppState>>,
    Path((_repo, run_id)): Path<(String, i64)>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    match grpc.list_artifacts(run_id).await {
        Ok(resp) => {
            let artifacts: Vec<ArtifactJson> = resp.artifacts.into_iter().map(|a| ArtifactJson {
                id: a.id, run_id: a.run_id, name: a.name,
                size_bytes: a.size_bytes, created_at: a.created_at,
            }).collect();
            (StatusCode::OK, Json(artifacts)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

pub async fn list_releases(
    State(state): State<Arc<AppState>>,
    Path(repo): Path<String>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    match grpc.list_releases(&repo).await {
        Ok(resp) => {
            let releases: Vec<ReleaseJson> = resp.releases.into_iter().map(|r| {
                let artifacts = r.artifacts.into_iter().map(|a| ArtifactJson {
                    id: a.id, run_id: a.run_id, name: a.name,
                    size_bytes: a.size_bytes, created_at: a.created_at,
                }).collect();
                ReleaseJson {
                    id: r.id, tag: r.tag, name: r.name,
                    run_id: r.run_id, created_at: r.created_at, artifacts,
                }
            }).collect();
            (StatusCode::OK, Json(releases)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

pub async fn get_release(
    State(state): State<Arc<AppState>>,
    Path((_repo, release_id)): Path<(String, i64)>,
) -> Response {
    let grpc = match state.grpc_client().await {
        Ok(g) => g,
        Err(e) => return internal_error(e),
    };
    match grpc.get_release(release_id).await {
        Ok(resp) => {
            if let Some(r) = resp.release {
                let artifacts = r.artifacts.into_iter().map(|a| ArtifactJson {
                    id: a.id, run_id: a.run_id, name: a.name,
                    size_bytes: a.size_bytes, created_at: a.created_at,
                }).collect();
                let release = ReleaseJson {
                    id: r.id, tag: r.tag, name: r.name,
                    run_id: r.run_id, created_at: r.created_at, artifacts,
                };
                (StatusCode::OK, Json(release)).into_response()
            } else {
                (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Release not found"}))).into_response()
            }
        }
        Err(e) => internal_error(e),
    }
}
