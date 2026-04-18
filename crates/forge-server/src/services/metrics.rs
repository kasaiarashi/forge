// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! `/metrics` + `/healthz` + `/readyz` endpoints (Phase 7).
//!
//! Exposed on a separate HTTP port (default `127.0.0.1:9877`) so a
//! sidecar Prometheus / k8s probe can hit the server without needing
//! the gRPC TLS trust chain. Three endpoints:
//!
//! - `GET /healthz` — liveness. Always `200 OK` when the HTTP task
//!   is scheduling; indicates the process is up. Use for k8s
//!   liveness probes.
//! - `GET /readyz` — readiness. `200 OK` only when the metadata DB
//!   responds to `SELECT 1`. A 503 means the server is up but can't
//!   serve traffic (pool exhausted, DB file corrupt). Use for k8s
//!   readiness probes and load-balancer drain.
//! - `GET /metrics` — Prometheus text format 0.0.4.
//!
//! **On-scrape counts.** The gauges are queried from the DB every
//! scrape rather than maintained as live counters. Scraping runs
//! 4× `SELECT COUNT(*)` on small indexed tables — negligible cost
//! vs the wiring burden of incrementing counters from every handler.
//! Per-method gRPC counters land in a follow-up if operator feedback
//! needs them.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use tracing::{info, warn};

use crate::storage::db::MetadataDb;

/// Shared state handed to each axum handler. Carries the DB handle
/// + the server's startup instant so `/metrics` can report
/// `forge_server_uptime_seconds`.
#[derive(Clone)]
pub struct MetricsState {
    pub db: Arc<MetadataDb>,
    pub start: Instant,
    pub version: &'static str,
}

/// Spawn the listener on the given address. Returns immediately;
/// failures to bind land in the log but don't take down the server.
pub fn spawn(state: MetricsState, listen: String) {
    tokio::spawn(async move {
        let addr: SocketAddr = match listen.parse() {
            Ok(a) => a,
            Err(e) => {
                warn!(listen = %listen, error = %e, "metrics listen address invalid, endpoint disabled");
                return;
            }
        };
        let app = Router::new()
            .route("/healthz", get(healthz))
            .route("/readyz", get(readyz))
            .route("/metrics", get(metrics))
            .with_state(state);

        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                warn!(
                    listen = %addr, error = %e,
                    "metrics listener bind failed, endpoint disabled"
                );
                return;
            }
        };
        info!("metrics + health endpoints listening on http://{}", addr);
        if let Err(e) = axum::serve(listener, app).await {
            warn!(error = %e, "metrics listener exited");
        }
    });
}

async fn healthz() -> impl IntoResponse {
    // Liveness: always healthy if we can execute this handler.
    (StatusCode::OK, "ok\n")
}

async fn readyz(
    axum::extract::State(state): axum::extract::State<MetricsState>,
) -> impl IntoResponse {
    // Readiness: DB must respond. If the pool or file is wedged we
    // prefer returning 503 over pretending to be healthy and
    // serving failing pushes.
    match tokio::task::spawn_blocking(move || state.db.ping()).await {
        Ok(Ok(())) => (StatusCode::OK, "ready\n").into_response(),
        Ok(Err(e)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("db not ready: {e}\n"),
        )
            .into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("readiness probe panicked: {e}\n"),
        )
            .into_response(),
    }
}

async fn metrics(
    axum::extract::State(state): axum::extract::State<MetricsState>,
) -> impl IntoResponse {
    // All counts fetched in one blocking task so we don't hop threads
    // per query.
    let start = state.start;
    let version = state.version;
    let db = Arc::clone(&state.db);
    let snapshot = tokio::task::spawn_blocking(move || db.metrics_snapshot()).await;

    let body = match snapshot {
        Ok(Ok(snap)) => render_prom(&snap, start.elapsed().as_secs_f64(), version),
        Ok(Err(e)) => {
            // DB error — still emit build_info + uptime + a gauge for
            // the failure so operators see it on the dashboard.
            let mut s = render_header(start.elapsed().as_secs_f64(), version);
            s.push_str("# HELP forge_server_metrics_error 1 when the last scrape failed to query the DB\n");
            s.push_str("# TYPE forge_server_metrics_error gauge\n");
            s.push_str(&format!("forge_server_metrics_error 1 # {e}\n"));
            s
        }
        Err(e) => {
            let mut s = render_header(start.elapsed().as_secs_f64(), version);
            s.push_str(&format!("forge_server_metrics_error 1 # panic: {e}\n"));
            s
        }
    };

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
}

fn render_header(uptime_secs: f64, version: &str) -> String {
    // Prometheus text format 0.0.4 (https://prometheus.io/docs/instrumenting/exposition_formats/).
    // Each metric: HELP line + TYPE line + one or more sample lines.
    let mut s = String::with_capacity(1024);
    s.push_str("# HELP forge_server_build_info Build info; label set carries the version.\n");
    s.push_str("# TYPE forge_server_build_info gauge\n");
    s.push_str(&format!("forge_server_build_info{{version=\"{version}\"}} 1\n"));

    s.push_str("# HELP forge_server_uptime_seconds Seconds since process start.\n");
    s.push_str("# TYPE forge_server_uptime_seconds gauge\n");
    s.push_str(&format!("forge_server_uptime_seconds {uptime_secs:.3}\n"));
    s
}

fn render_prom(
    snap: &crate::storage::db::MetricsSnapshot,
    uptime_secs: f64,
    version: &str,
) -> String {
    let mut s = render_header(uptime_secs, version);

    s.push_str("# HELP forge_server_upload_sessions_uploading Upload sessions currently streaming chunks.\n");
    s.push_str("# TYPE forge_server_upload_sessions_uploading gauge\n");
    s.push_str(&format!(
        "forge_server_upload_sessions_uploading {}\n",
        snap.uploading_sessions,
    ));

    s.push_str("# HELP forge_server_locks_total Active file locks across all repos.\n");
    s.push_str("# TYPE forge_server_locks_total gauge\n");
    s.push_str(&format!("forge_server_locks_total {}\n", snap.total_locks));

    s.push_str("# HELP forge_server_repos_total Repositories the server knows about.\n");
    s.push_str("# TYPE forge_server_repos_total gauge\n");
    s.push_str(&format!("forge_server_repos_total {}\n", snap.total_repos));

    s.push_str("# HELP forge_server_pending_repo_ops Pending S3 rename/delete drain ops.\n");
    s.push_str("# TYPE forge_server_pending_repo_ops gauge\n");
    s.push_str(&format!(
        "forge_server_pending_repo_ops {}\n",
        snap.pending_repo_ops,
    ));

    s.push_str("# HELP forge_server_metrics_error 1 when the last scrape failed to query the DB\n");
    s.push_str("# TYPE forge_server_metrics_error gauge\n");
    s.push_str("forge_server_metrics_error 0\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::MetricsSnapshot;

    #[test]
    fn render_prom_emits_valid_text_format() {
        let snap = MetricsSnapshot {
            uploading_sessions: 3,
            total_locks: 42,
            total_repos: 7,
            pending_repo_ops: 1,
        };
        let out = render_prom(&snap, 120.5, "0.4.0");

        // Required header hygiene: every metric has a HELP + TYPE line.
        let required = [
            "# HELP forge_server_build_info",
            "# TYPE forge_server_build_info gauge",
            "forge_server_build_info{version=\"0.4.0\"} 1",
            "# HELP forge_server_uptime_seconds",
            "# TYPE forge_server_uptime_seconds gauge",
            "forge_server_uptime_seconds 120.500",
            "forge_server_upload_sessions_uploading 3",
            "forge_server_locks_total 42",
            "forge_server_repos_total 7",
            "forge_server_pending_repo_ops 1",
            "forge_server_metrics_error 0",
        ];
        for want in required {
            assert!(
                out.contains(want),
                "output missing `{want}`; got:\n{out}",
            );
        }
    }

    #[test]
    fn render_header_survives_db_error() {
        // Even with zero metrics we emit build_info + uptime so the
        // scraper's dashboard doesn't go blank on a transient DB
        // hiccup.
        let out = render_header(0.0, "0.0.0");
        assert!(out.contains("forge_server_build_info{version=\"0.0.0\"} 1"));
        assert!(out.contains("forge_server_uptime_seconds 0.000"));
    }
}
