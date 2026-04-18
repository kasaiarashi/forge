// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under BSL 1.1.

//! Phase 7f — closed-loop gRPC load harness.
//!
//! Spawns N concurrent virtual users against an existing forge-server,
//! each running a weighted workload mix (lock acquire/release, list
//! locks, ref reads). Aggregates per-RPC latency into HDR histograms
//! and prints p50/p95/p99 + RPS at the end.
//!
//! This is the harness side of the "500-user load test" deliverable —
//! the threshold story (what's a regression?) is captured on the
//! progress tracker and refined as we collect baselines.
//!
//! Invoke via the binary:
//! ```text
//! forge-server load-test \
//!     --target https://localhost:50051 \
//!     --token <PAT> \
//!     --repo <owner>/<name> \
//!     --users 50 --duration 30
//! ```
//!
//! Trust resolution mirrors `forge` CLI conventions: `FORGE_CA_CERT`
//! env var → optional `--ca-cert` flag → system roots. For a fully
//! self-signed loopback test, `--insecure` skips verification — never
//! use that against a production server.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use hdrhistogram::Histogram;
use tokio::sync::Mutex;
use tonic::metadata::MetadataValue;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};
use tonic::{Request, Status};

use forge_proto::forge::forge_service_client::ForgeServiceClient;
use forge_proto::forge::{GetRefsRequest, ListLocksRequest, LockRequest, UnlockRequest};

/// Knobs for a single load run. Bundled into a struct so the binary
/// CLI parsing layer doesn't smear across fn signatures.
#[derive(Debug, Clone)]
pub struct LoadTestConfig {
    pub target: String,
    pub token: Option<String>,
    pub repo: String,
    pub users: usize,
    pub duration: Duration,
    pub ca_cert: Option<PathBuf>,
    pub insecure: bool,
    /// Workspace identifier reused across all sim users. Real clients
    /// pin a single workspace per machine — N virtual users sharing
    /// one workspace_id is fine because the lock paths are unique.
    pub workspace_id: String,
}

/// Per-RPC latency aggregate. Histograms record microseconds; the
/// 1-second cap is plenty since anything past that is a server stall.
#[derive(Debug)]
struct OpStats {
    histogram: Histogram<u64>,
    successes: u64,
    failures: u64,
}

impl OpStats {
    fn new() -> Self {
        Self {
            // 3 sig figs, ceiling 60 s — gives us p99 fidelity through
            // tail stalls without burning megs of memory per op.
            histogram: Histogram::<u64>::new_with_max(60_000_000, 3)
                .expect("hdrhistogram bounds"),
            successes: 0,
            failures: 0,
        }
    }

    fn record(&mut self, latency: Duration, ok: bool) {
        let micros = latency.as_micros().min(60_000_000) as u64;
        // Saturate at the histogram ceiling rather than panic if a
        // pathological outlier slips past the min().
        let _ = self.histogram.record(micros);
        if ok {
            self.successes += 1;
        } else {
            self.failures += 1;
        }
    }
}

/// Aggregate report across all virtual users + ops.
#[derive(Debug)]
pub struct LoadTestReport {
    pub elapsed: Duration,
    pub per_op: Vec<(String, ReportRow)>,
    pub total_ops: u64,
    pub total_failures: u64,
}

#[derive(Debug, Clone)]
pub struct ReportRow {
    pub count: u64,
    pub failures: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
}

/// Run the load test to completion. Blocks the caller's tokio runtime
/// until `duration` elapses. The N user tasks are spawned on the
/// current runtime so the test inherits whatever multi-thread shape
/// the operator picked.
pub async fn run(cfg: LoadTestConfig) -> Result<LoadTestReport> {
    if cfg.users == 0 {
        anyhow::bail!("users must be >= 1");
    }
    if cfg.duration.is_zero() {
        anyhow::bail!("duration must be > 0");
    }

    let endpoint = build_endpoint(&cfg)?;
    // Connect once per user task — separate channels stress the server
    // connection-handling path the way real distributed clients do,
    // rather than multiplexing every user through one HTTP/2 link.
    println!(
        "load-test: target={} users={} duration={}s repo={}",
        cfg.target,
        cfg.users,
        cfg.duration.as_secs(),
        cfg.repo,
    );

    let acquire_stats = Arc::new(Mutex::new(OpStats::new()));
    let release_stats = Arc::new(Mutex::new(OpStats::new()));
    let list_stats = Arc::new(Mutex::new(OpStats::new()));
    let get_ref_stats = Arc::new(Mutex::new(OpStats::new()));

    let deadline = Instant::now() + cfg.duration;
    let mut handles = Vec::with_capacity(cfg.users);
    let start = Instant::now();

    for user_id in 0..cfg.users {
        let endpoint = endpoint.clone();
        let cfg = cfg.clone();
        let acquire_stats = Arc::clone(&acquire_stats);
        let release_stats = Arc::clone(&release_stats);
        let list_stats = Arc::clone(&list_stats);
        let get_ref_stats = Arc::clone(&get_ref_stats);
        handles.push(tokio::spawn(async move {
            run_user(
                user_id,
                endpoint,
                cfg,
                deadline,
                acquire_stats,
                release_stats,
                list_stats,
                get_ref_stats,
            )
            .await
        }));
    }

    // Drain user tasks. A panicking user task aborts the run because a
    // connection-level fault should not be silently dropped from the
    // aggregate.
    for h in handles {
        if let Err(e) = h.await {
            anyhow::bail!("user task panicked: {e}");
        }
    }

    let elapsed = start.elapsed();

    let per_op = vec![
        ("acquire_lock".to_string(), summarize(&*acquire_stats.lock().await)),
        ("release_lock".to_string(), summarize(&*release_stats.lock().await)),
        ("list_locks".to_string(), summarize(&*list_stats.lock().await)),
        ("get_ref".to_string(), summarize(&*get_ref_stats.lock().await)),
    ];

    let total_ops = per_op.iter().map(|(_, r)| r.count).sum();
    let total_failures = per_op.iter().map(|(_, r)| r.failures).sum();

    Ok(LoadTestReport {
        elapsed,
        per_op,
        total_ops,
        total_failures,
    })
}

fn summarize(stats: &OpStats) -> ReportRow {
    let h = &stats.histogram;
    ReportRow {
        count: stats.successes + stats.failures,
        failures: stats.failures,
        p50_us: h.value_at_quantile(0.50),
        p95_us: h.value_at_quantile(0.95),
        p99_us: h.value_at_quantile(0.99),
        max_us: h.max(),
    }
}

/// Pretty-print `report` to stdout. The shape matches what an operator
/// would paste into a perf-regression bug.
pub fn print_report(report: &LoadTestReport) {
    println!();
    println!(
        "load-test complete: elapsed={:.2}s total_ops={} failures={} rps={:.1}",
        report.elapsed.as_secs_f64(),
        report.total_ops,
        report.total_failures,
        report.total_ops as f64 / report.elapsed.as_secs_f64(),
    );
    println!();
    println!(
        "{:<14} {:>10} {:>8} {:>10} {:>10} {:>10} {:>10}",
        "op", "count", "fail", "p50_us", "p95_us", "p99_us", "max_us",
    );
    println!("{}", "-".repeat(80));
    for (name, row) in &report.per_op {
        println!(
            "{:<14} {:>10} {:>8} {:>10} {:>10} {:>10} {:>10}",
            name, row.count, row.failures, row.p50_us, row.p95_us, row.p99_us, row.max_us,
        );
    }
}

fn build_endpoint(cfg: &LoadTestConfig) -> Result<Endpoint> {
    let endpoint = Endpoint::from_shared(cfg.target.clone())
        .with_context(|| format!("invalid target url '{}'", cfg.target))?
        .initial_connection_window_size(16 * 1024 * 1024)
        .initial_stream_window_size(16 * 1024 * 1024)
        .http2_adaptive_window(true)
        .tcp_nodelay(true);
    if !cfg.target.starts_with("https://") {
        return Ok(endpoint);
    }

    if cfg.insecure {
        // tonic doesn't expose the rustls "danger" hooks; the
        // operator must point us at a CA file via `--ca-cert` (or
        // `FORGE_CA_CERT`) for now. Fail loudly so they don't think
        // they're getting unverified TLS.
        anyhow::bail!(
            "--insecure for HTTPS targets is not supported by the tonic ClientTlsConfig \
             surface — pass --ca-cert <PEM> with the server's CA, or set FORGE_CA_CERT."
        );
    }

    let pem = if let Some(path) = &cfg.ca_cert {
        std::fs::read(path)
            .with_context(|| format!("failed to read --ca-cert {}", path.display()))?
    } else if let Ok(env_path) = std::env::var("FORGE_CA_CERT") {
        std::fs::read(&env_path)
            .with_context(|| format!("failed to read FORGE_CA_CERT={env_path}"))?
    } else {
        // Fall back to system roots — ok for production CAs.
        let tls = ClientTlsConfig::new().with_native_roots();
        return endpoint.tls_config(tls).context("tls config");
    };
    let tls = ClientTlsConfig::new().ca_certificate(Certificate::from_pem(pem));
    endpoint.tls_config(tls).context("tls config")
}

#[derive(Clone)]
struct AuthInterceptor {
    token: Option<String>,
}

impl tonic::service::Interceptor for AuthInterceptor {
    fn call(&mut self, mut req: Request<()>) -> Result<Request<()>, Status> {
        if let Some(t) = &self.token {
            let v = format!("Bearer {t}")
                .parse::<MetadataValue<_>>()
                .map_err(|e| Status::internal(format!("bad token: {e}")))?;
            req.metadata_mut().insert("authorization", v);
        }
        Ok(req)
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_user(
    user_id: usize,
    endpoint: Endpoint,
    cfg: LoadTestConfig,
    deadline: Instant,
    acquire_stats: Arc<Mutex<OpStats>>,
    release_stats: Arc<Mutex<OpStats>>,
    list_stats: Arc<Mutex<OpStats>>,
    get_ref_stats: Arc<Mutex<OpStats>>,
) {
    let channel = match endpoint.connect().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(user_id, error = %e, "user connect failed; aborting");
            return;
        }
    };
    let interceptor = AuthInterceptor { token: cfg.token.clone() };
    let mut client = ForgeServiceClient::with_interceptor(channel, interceptor);

    // Per-user counter sequences a unique lock path on every iteration
    // so two virtual users never fight over the same row. Without
    // this, the contention skew would dominate every reading and
    // hide pool/CAS regressions we actually want to measure.
    let mut iter: u64 = 0;
    let owner = format!("loadtest-u{user_id}");

    while Instant::now() < deadline {
        // Workload mix (rough P4-team weights):
        // 50% list_locks  20% get_ref  30% acquire+release pair
        let pick = iter % 10;
        match pick {
            0..=4 => {
                let t0 = Instant::now();
                let res = client
                    .list_locks(ListLocksRequest {
                        repo: cfg.repo.clone(),
                        path_prefix: String::new(),
                        owner: String::new(),
                    })
                    .await;
                list_stats
                    .lock()
                    .await
                    .record(t0.elapsed(), res.is_ok());
            }
            5..=6 => {
                let t0 = Instant::now();
                let res = client
                    .get_refs(GetRefsRequest {
                        repo: cfg.repo.clone(),
                    })
                    .await;
                get_ref_stats
                    .lock()
                    .await
                    .record(t0.elapsed(), res.is_ok());
            }
            _ => {
                let path = format!("Content/loadtest/u{user_id}/lock{iter}.umap");
                let t0 = Instant::now();
                let acquired = client
                    .acquire_lock(LockRequest {
                        repo: cfg.repo.clone(),
                        path: path.clone(),
                        owner: owner.clone(),
                        workspace_id: cfg.workspace_id.clone(),
                        reason: "load test".to_string(),
                    })
                    .await;
                let acquire_ok = acquired.is_ok();
                acquire_stats
                    .lock()
                    .await
                    .record(t0.elapsed(), acquire_ok);

                if acquire_ok {
                    let t1 = Instant::now();
                    let released = client
                        .release_lock(UnlockRequest {
                            repo: cfg.repo.clone(),
                            path,
                            owner: owner.clone(),
                            force: false,
                        })
                        .await;
                    release_stats
                        .lock()
                        .await
                        .record(t1.elapsed(), released.is_ok());
                }
            }
        }

        iter += 1;
    }
}

// Placate the `unused` lint when downstream code (the binary's CLI
// dispatch) is the only consumer.
#[allow(dead_code)]
type _SuppressInterceptedService = InterceptedService<Channel, AuthInterceptor>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opstats_records_quantiles() {
        let mut s = OpStats::new();
        for us in 1..=1000u64 {
            s.record(Duration::from_micros(us), us % 7 != 0);
        }
        let row = summarize(&s);
        assert_eq!(row.count, 1000);
        // Failures = ceil(1000/7) = 142.
        assert_eq!(row.failures, 142);
        assert!(row.p50_us > 0 && row.p50_us < row.p99_us);
        assert!(row.p99_us <= row.max_us);
    }

    #[test]
    fn run_rejects_zero_users() {
        let cfg = LoadTestConfig {
            target: "http://127.0.0.1:1".to_string(),
            token: None,
            repo: "x/y".to_string(),
            users: 0,
            duration: Duration::from_secs(1),
            ca_cert: None,
            insecure: false,
            workspace_id: "ws".to_string(),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt.block_on(run(cfg)).unwrap_err();
        assert!(err.to_string().contains("users"));
    }

    #[test]
    fn run_rejects_zero_duration() {
        let cfg = LoadTestConfig {
            target: "http://127.0.0.1:1".to_string(),
            token: None,
            repo: "x/y".to_string(),
            users: 1,
            duration: Duration::ZERO,
            ca_cert: None,
            insecure: false,
            workspace_id: "ws".to_string(),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt.block_on(run(cfg)).unwrap_err();
        assert!(err.to_string().contains("duration"));
    }
}
