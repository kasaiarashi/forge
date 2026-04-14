// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! `AgentService` — distributed runner gRPC endpoint.
//!
//! An agent registers with a pre-issued token (plaintext on the wire over
//! TLS; Argon2-hashed on the server), then long-polls `ClaimJob` to pick up
//! queued workflow runs. It reports step outcomes and streams step logs
//! back over the same TLS channel. Secret resolution and action YAML
//! fetches are scoped to runs the agent has actually claimed — an agent
//! can't probe for secrets it isn't entitled to.

use std::sync::Arc;
use std::time::Duration;
use tonic::{Request, Response, Status};

use argon2::{password_hash::{PasswordHash, PasswordVerifier}, Argon2};
use forge_proto::forge::agent_service_server::AgentService;
use forge_proto::forge::*;

use crate::services::logs::{LogChunk, LogHub};
use crate::services::secrets::SecretBackend;
use crate::storage::db::MetadataDb;

pub struct ForgeAgentService {
    pub db: Arc<MetadataDb>,
    pub secrets: Arc<dyn SecretBackend>,
    pub log_hub: Arc<LogHub>,
}

impl ForgeAgentService {
    /// Verify a (name, token) pair against the stored Argon2 hash. All
    /// agent RPCs route through this before any real work.
    fn authenticate(&self, agent_id: i64, token: &str) -> Result<String, Status> {
        let (name, token_hash, _labels) = self
            .db
            .get_agent_by_id(agent_id)
            .map_err(|e| {
                tracing::error!(error = %e, "agent lookup");
                Status::internal("internal server error")
            })?
            .ok_or_else(|| Status::unauthenticated("unknown agent"))?;
        let parsed = PasswordHash::new(&token_hash)
            .map_err(|_| Status::internal("corrupt agent hash"))?;
        Argon2::default()
            .verify_password(token.as_bytes(), &parsed)
            .map_err(|_| Status::unauthenticated("invalid agent token"))?;
        Ok(name)
    }
}

#[tonic::async_trait]
impl AgentService for ForgeAgentService {
    async fn register_agent(
        &self,
        request: Request<RegisterAgentRequest>,
    ) -> Result<Response<RegisterAgentResponse>, Status> {
        let req = request.into_inner();
        // Match by name; the server-side `forge-server agent add` call
        // wrote the Argon2 hash of `req.token`. If name exists, verify
        // token; if not, refuse (no implicit registration — tokens must
        // be provisioned first).
        let (agent_id, token_hash, _) = self
            .db
            .get_agent_by_name(&req.name)
            .map_err(|e| {
                tracing::error!(error = %e, "agent lookup");
                Status::internal("internal server error")
            })?
            .ok_or_else(|| Status::unauthenticated("unknown agent name"))?;
        let parsed = PasswordHash::new(&token_hash)
            .map_err(|_| Status::internal("corrupt agent hash"))?;
        Argon2::default()
            .verify_password(req.token.as_bytes(), &parsed)
            .map_err(|_| Status::unauthenticated("invalid agent token"))?;

        // Refresh labels / version / os / last_seen.
        let labels_json = serde_json::to_string(&req.labels).unwrap_or_else(|_| "[]".into());
        self.db
            .upsert_agent(&req.name, &token_hash, &labels_json, &req.version, &req.os)
            .map_err(|e| {
                tracing::error!(error = %e, "agent upsert");
                Status::internal("internal server error")
            })?;

        Ok(Response::new(RegisterAgentResponse {
            agent_id,
            heartbeat_seconds: 15,
            claim_poll_seconds: 30,
        }))
    }

    async fn heartbeat(
        &self,
        request: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        let req = request.into_inner();
        self.authenticate(req.agent_id, &req.token)?;
        self.db
            .touch_agent_last_seen(req.agent_id)
            .map_err(|e| {
                tracing::error!(error = %e, "agent touch");
                Status::internal("internal server error")
            })?;
        Ok(Response::new(HeartbeatResponse { ok: true }))
    }

    async fn claim_job(
        &self,
        request: Request<ClaimJobRequest>,
    ) -> Result<Response<ClaimJobResponse>, Status> {
        let req = request.into_inner();
        let _name = self.authenticate(req.agent_id, &req.token)?;
        let wait = Duration::from_secs(req.wait_seconds.clamp(5, 60) as u64);

        // Poll the DB a few times over the wait window. A full long-poll
        // with DB NOTIFY would be nicer but SQLite doesn't do that;
        // 1-second tick-with-backoff is cheap enough.
        let start = std::time::Instant::now();
        loop {
            let labels = self
                .db
                .get_agent_by_id(req.agent_id)
                .ok()
                .flatten()
                .map(|(_, _, lj)| {
                    serde_json::from_str::<Vec<String>>(&lj).unwrap_or_default()
                })
                .unwrap_or_default();
            if let Some(run_id) = self
                .db
                .claim_next_run(req.agent_id, &labels)
                .map_err(|e| {
                    tracing::error!(error = %e, "claim_next_run");
                    Status::internal("internal server error")
                })?
            {
                let run = self
                    .db
                    .get_run(run_id)
                    .map_err(|e| {
                        tracing::error!(error = %e, "get_run after claim");
                        Status::internal("internal server error")
                    })?
                    .ok_or_else(|| Status::internal("claimed run vanished"))?;
                let wf = self
                    .db
                    .get_workflow(run.workflow_id)
                    .map_err(|e| {
                        tracing::error!(error = %e, "get_workflow after claim");
                        Status::internal("internal server error")
                    })?
                    .ok_or_else(|| Status::internal("workflow gone"))?;

                // Resolve secrets for the run's repo once and hand the
                // plaintext env to the agent. Over TLS; no other channel.
                let resolved_env =
                    resolve_env(&wf.yaml, &run.repo, self.secrets.as_ref())
                        .await
                        .map_err(|e| {
                            tracing::error!(error = %e, "resolve env");
                            Status::internal("internal server error")
                        })?;

                return Ok(Response::new(ClaimJobResponse {
                    run_id,
                    repo: run.repo,
                    workflow_yaml: wf.yaml,
                    commit_hash: run.commit_hash,
                    trigger_ref: run.trigger_ref,
                    env: resolved_env,
                }));
            }
            if start.elapsed() >= wait {
                return Ok(Response::new(ClaimJobResponse::default()));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn report_step(
        &self,
        request: Request<ReportStepRequest>,
    ) -> Result<Response<ReportStepResponse>, Status> {
        let req = request.into_inner();
        self.authenticate(req.agent_id, &req.token)?;

        // Agent must own this run.
        let owner = self
            .db
            .get_run_claim_agent(req.run_id)
            .map_err(|e| {
                tracing::error!(error = %e, "claim owner");
                Status::internal("internal server error")
            })?;
        if owner != Some(req.agent_id) {
            return Err(Status::permission_denied("agent does not own this run"));
        }

        let step_id = self
            .db
            .create_step(req.run_id, &req.job_name, req.step_index, &req.name)
            .map_err(|e| {
                tracing::error!(error = %e, "create_step");
                Status::internal("internal server error")
            })?;
        self.db
            .update_step(step_id, &req.status, Some(req.exit_code), &req.log_tail)
            .map_err(|e| {
                tracing::error!(error = %e, "update_step");
                Status::internal("internal server error")
            })?;
        Ok(Response::new(ReportStepResponse { ok: true }))
    }

    async fn stream_agent_logs(
        &self,
        request: Request<tonic::Streaming<AgentLogChunk>>,
    ) -> Result<Response<StreamAgentLogsResponse>, Status> {
        let mut stream = request.into_inner();
        // First chunk must authenticate. We re-auth on every chunk so a
        // rotated token takes effect immediately.
        while let Some(chunk) = stream.message().await? {
            self.authenticate(chunk.agent_id, &chunk.token)?;
            let owner = self
                .db
                .get_run_claim_agent(chunk.run_id)
                .map_err(|_| Status::internal("claim lookup"))?;
            if owner != Some(chunk.agent_id) {
                return Err(Status::permission_denied("agent does not own this run"));
            }
            let sender = self.log_hub.sender(chunk.run_id);
            let _ = sender.send(LogChunk {
                run_id: chunk.run_id,
                step_id: chunk.step_id,
                data: chunk.data,
                is_final: chunk.is_final,
            });
        }
        Ok(Response::new(StreamAgentLogsResponse { ok: true }))
    }

    async fn report_run_finished(
        &self,
        request: Request<ReportRunFinishedRequest>,
    ) -> Result<Response<ReportRunFinishedResponse>, Status> {
        let req = request.into_inner();
        self.authenticate(req.agent_id, &req.token)?;
        let owner = self
            .db
            .get_run_claim_agent(req.run_id)
            .map_err(|_| Status::internal("claim lookup"))?;
        if owner != Some(req.agent_id) {
            return Err(Status::permission_denied("agent does not own this run"));
        }
        self.db
            .update_run_status(req.run_id, &req.status)
            .map_err(|e| {
                tracing::error!(error = %e, "update_run_status");
                Status::internal("internal server error")
            })?;
        self.log_hub.close(req.run_id);
        Ok(Response::new(ReportRunFinishedResponse { ok: true }))
    }

    async fn get_run_secret(
        &self,
        request: Request<GetRunSecretRequest>,
    ) -> Result<Response<GetRunSecretResponse>, Status> {
        let req = request.into_inner();
        self.authenticate(req.agent_id, &req.token)?;
        let owner = self
            .db
            .get_run_claim_agent(req.run_id)
            .map_err(|_| Status::internal("claim lookup"))?;
        if owner != Some(req.agent_id) {
            return Err(Status::permission_denied("agent does not own this run"));
        }
        let run = self
            .db
            .get_run(req.run_id)
            .map_err(|_| Status::internal("run lookup"))?
            .ok_or_else(|| Status::not_found("run not found"))?;
        let wf = self
            .db
            .get_workflow(run.workflow_id)
            .map_err(|_| Status::internal("workflow lookup"))?
            .ok_or_else(|| Status::not_found("workflow gone"))?;
        // Reject if the workflow doesn't reference this secret — a claimed
        // agent still can't enumerate unrelated secrets.
        let ref_pattern = format!("secrets.{}", req.key);
        if !wf.yaml.contains(&ref_pattern) {
            return Err(Status::permission_denied(
                "workflow does not reference this secret",
            ));
        }
        let s = self
            .secrets
            .get(&run.repo, &req.key)
            .await
            .map_err(|_| Status::internal("secret lookup"))?
            .ok_or_else(|| Status::not_found("secret not set"))?;
        Ok(Response::new(GetRunSecretResponse { value: s.value }))
    }

    async fn get_action(
        &self,
        _request: Request<GetActionRequest>,
    ) -> Result<Response<GetActionResponse>, Status> {
        // Phase 3 wires this up to the composite action registry; for now
        // return empty so an agent can still run `run:`-only jobs without
        // crashing on missing actions.
        Ok(Response::new(GetActionResponse { yaml: String::new() }))
    }
}

async fn resolve_env(
    yaml: &str,
    repo: &str,
    secrets: &dyn SecretBackend,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let def: crate::services::actions::yaml::WorkflowDef =
        serde_yaml::from_str(yaml)?;
    let re = regex::Regex::new(r"\$\{\{\s*secrets\.([A-Za-z_][A-Za-z0-9_]*)\s*\}\}")
        .expect("static regex");
    let mut out = std::collections::HashMap::with_capacity(def.env.len());
    let mut cache = std::collections::HashMap::<String, String>::new();
    for (k, v) in &def.env {
        let mut expanded = v.clone();
        for cap in re.captures_iter(v) {
            let name = cap[1].to_string();
            let val = match cache.get(&name) {
                Some(v) => v.clone(),
                None => {
                    let s = secrets.get(repo, &name).await?.ok_or_else(|| {
                        anyhow::anyhow!("secret '{}' referenced but not set", name)
                    })?;
                    cache.insert(name.clone(), s.value.clone());
                    s.value
                }
            };
            expanded = expanded.replace(&cap[0], &val);
        }
        out.insert(k.clone(), expanded);
    }
    Ok(out)
}
