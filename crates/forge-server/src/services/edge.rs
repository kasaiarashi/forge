// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under BSL 1.1.

//! Phase 7e — read-only edge replica.
//!
//! A `forge-server serve --read-only` instance accepts the full
//! gRPC surface but rejects every write RPC at the tower layer with
//! `FailedPrecondition`. The error message carries the upstream
//! write URL so a smart client can transparently re-route.
//!
//! Operators replicate the metadata DB (Litestream → S3) and the
//! object store (rsync / S3 sync / hardlink farm) onto each edge
//! host. The edge serves all read-only RPCs — pulls, has-checks, ref
//! reads, lock list, browser endpoints — out of its local replica
//! while writes are funneled to the single primary.
//!
//! This file ships only the request gate; the replication side is a
//! pure-ops deliverable (see `docs/ha/litestream.yml.example` for
//! the SQLite WAL stream and `docs/ha/edge.md` for the recommended
//! object-side mirroring story).

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tonic::body::BoxBody;
use tonic::codegen::http::{Request as HttpRequest, Response as HttpResponse};
use tonic::Status;
use tower::{Layer, Service};

/// Every gRPC method path that performs a write. Anything not in
/// here passes through unchanged. We list paths explicitly rather
/// than match by HTTP verb because gRPC is always POST — the only
/// way to tell read from write is the method name.
fn write_paths() -> HashSet<&'static str> {
    let mut s = HashSet::new();
    // ForgeService writes
    for p in [
        "/forge.ForgeService/PushObjects",
        "/forge.ForgeService/CommitPush",
        "/forge.ForgeService/UpdateRef",
        "/forge.ForgeService/AcquireLock",
        "/forge.ForgeService/ReleaseLock",
        "/forge.ForgeService/CreateRepo",
        "/forge.ForgeService/UpdateRepo",
        "/forge.ForgeService/DeleteRepo",
        "/forge.ForgeService/CreateWorkflow",
        "/forge.ForgeService/UpdateWorkflow",
        "/forge.ForgeService/DeleteWorkflow",
        "/forge.ForgeService/TriggerWorkflow",
        "/forge.ForgeService/CancelWorkflowRun",
        "/forge.ForgeService/UploadArtifact",
        "/forge.ForgeService/CreateIssue",
        "/forge.ForgeService/UpdateIssue",
        "/forge.ForgeService/CreatePullRequest",
        "/forge.ForgeService/UpdatePullRequest",
        "/forge.ForgeService/MergePullRequest",
        "/forge.ForgeService/CreateComment",
        "/forge.ForgeService/UpdateComment",
        "/forge.ForgeService/DeleteComment",
        "/forge.ForgeService/CreateSecret",
        "/forge.ForgeService/UpdateSecret",
        "/forge.ForgeService/DeleteSecret",
    ] {
        s.insert(p);
    }
    // AuthService writes (account management). Login + WhoAmI stay
    // available so a user can still inspect their session against
    // an edge — but no PAT mint, no admin-mutation RPCs.
    for p in [
        "/forge.AuthService/CreatePersonalAccessToken",
        "/forge.AuthService/RevokePersonalAccessToken",
        "/forge.AuthService/RevokeSession",
        "/forge.AuthService/CreateUser",
        "/forge.AuthService/DeleteUser",
        "/forge.AuthService/GrantRepoRole",
        "/forge.AuthService/RevokeRepoRole",
        "/forge.AuthService/BootstrapAdmin",
    ] {
        s.insert(p);
    }
    // AgentService is write-heavy by definition (claim, report,
    // heartbeat). Edge replicas do not run the actions engine.
    for p in [
        "/forge.AgentService/RegisterAgent",
        "/forge.AgentService/Heartbeat",
        "/forge.AgentService/ClaimJob",
        "/forge.AgentService/ReportStep",
        "/forge.AgentService/StreamAgentLogs",
        "/forge.AgentService/ReportRunFinished",
    ] {
        s.insert(p);
    }
    s
}

/// tower layer that gates writes when running in read-only mode.
///
/// `upstream_hint` is included verbatim in the error message clients
/// see on a rejected write — operators set it to the primary's
/// public URL so smart clients (forge-cli ≥ next release) can
/// transparently retry against the right endpoint.
#[derive(Clone)]
pub struct ReadOnlyLayer {
    inner: Arc<ReadOnlyConfig>,
}

struct ReadOnlyConfig {
    write_paths: HashSet<&'static str>,
    upstream_hint: String,
}

impl ReadOnlyLayer {
    pub fn new(upstream_hint: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(ReadOnlyConfig {
                write_paths: write_paths(),
                upstream_hint: upstream_hint.into(),
            }),
        }
    }
}

impl<S> Layer<S> for ReadOnlyLayer {
    type Service = ReadOnlyService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ReadOnlyService {
            inner,
            cfg: Arc::clone(&self.inner),
        }
    }
}

#[derive(Clone)]
pub struct ReadOnlyService<S> {
    inner: S,
    cfg: Arc<ReadOnlyConfig>,
}

impl<S, B> Service<HttpRequest<B>> for ReadOnlyService<S>
where
    S: Service<HttpRequest<B>, Response = HttpResponse<BoxBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = HttpResponse<BoxBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: HttpRequest<B>) -> Self::Future {
        let path = req.uri().path().to_string();
        if self.cfg.write_paths.contains(path.as_str()) {
            let status = Status::failed_precondition(format!(
                "read-only edge replica refused write '{path}'; \
                 send writes to {}",
                self.cfg.upstream_hint,
            ));
            let resp = status.into_http();
            return Box::pin(async move { Ok(resp) });
        }
        // tower::Service requires `&mut self.inner` but the cloned
        // service is the contractually-correct way to call into it
        // from a `Pin<Box<dyn Future>>`. The clone is cheap — every
        // tonic generated service is `Arc`-shaped internally.
        let mut inner = self.inner.clone();
        Box::pin(async move { inner.call(req).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_paths_contain_known_writes() {
        let paths = write_paths();
        assert!(paths.contains("/forge.ForgeService/PushObjects"));
        assert!(paths.contains("/forge.ForgeService/AcquireLock"));
        assert!(paths.contains("/forge.AuthService/CreatePersonalAccessToken"));
        assert!(paths.contains("/forge.AgentService/ClaimJob"));
    }

    #[test]
    fn write_paths_excludes_reads() {
        let paths = write_paths();
        assert!(!paths.contains("/forge.ForgeService/PullObjects"));
        assert!(!paths.contains("/forge.ForgeService/HasObjects"));
        assert!(!paths.contains("/forge.ForgeService/GetRefs"));
        assert!(!paths.contains("/forge.ForgeService/ListLocks"));
        assert!(!paths.contains("/forge.AuthService/Login"));
        assert!(!paths.contains("/forge.AuthService/WhoAmI"));
    }
}
