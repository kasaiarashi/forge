// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use forge_proto::forge::auth_service_client::AuthServiceClient;
use forge_proto::forge::forge_service_client::ForgeServiceClient;
use forge_proto::forge::*;
use tonic::metadata::MetadataValue;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Status};

use crate::auth::current_session_token;

/// Wrapper around the tonic-generated gRPC client for forge-server.
///
/// Carries a single shared `Channel` (cheap to clone — just an Arc) and
/// constructs a fresh interceptor-wrapped client per call so each request
/// gets the right `Authorization` header. The session token comes from the
/// per-request tokio task-local set up by [`crate::auth::session_token_layer`].
#[derive(Clone)]
pub struct ForgeGrpcClient {
    channel: Channel,
}

impl ForgeGrpcClient {
    /// Connect to the forge-server at the given gRPC URL.
    pub async fn connect(grpc_url: &str) -> anyhow::Result<Self> {
        let channel = Endpoint::from_shared(grpc_url.to_string())?
            .connect()
            .await?;
        Ok(Self { channel })
    }

    /// Build a [`ForgeServiceClient`] for one request, attaching the current
    /// task-local session token (if any) as `Authorization: Bearer <token>`.
    fn forge(
        &self,
    ) -> ForgeServiceClient<InterceptedService<Channel, BearerInterceptor>> {
        ForgeServiceClient::with_interceptor(
            self.channel.clone(),
            BearerInterceptor::from_task_local(),
        )
    }

    /// Build an [`AuthServiceClient`] for one request, same shape.
    pub fn auth(
        &self,
    ) -> AuthServiceClient<InterceptedService<Channel, BearerInterceptor>> {
        AuthServiceClient::with_interceptor(
            self.channel.clone(),
            BearerInterceptor::from_task_local(),
        )
    }
}

/// Per-request tonic interceptor that injects the session bearer token
/// extracted by [`crate::auth::session_token_layer`].
#[derive(Clone, Default)]
pub struct BearerInterceptor {
    header: Option<MetadataValue<tonic::metadata::Ascii>>,
}

impl BearerInterceptor {
    fn from_task_local() -> Self {
        let header = current_session_token().and_then(|tok| {
            MetadataValue::try_from(format!("Bearer {tok}")).ok()
        });
        Self { header }
    }
}

impl tonic::service::Interceptor for BearerInterceptor {
    fn call(&mut self, mut req: Request<()>) -> Result<Request<()>, Status> {
        if let Some(ref h) = self.header {
            req.metadata_mut().insert("authorization", h.clone());
        }
        Ok(req)
    }
}

#[allow(dead_code)] // some methods are unused but kept symmetric with the gRPC surface
impl ForgeGrpcClient {

    /// List all repositories.
    pub async fn list_repos(&self) -> anyhow::Result<ListReposResponse> {
        let mut client = self.forge();
        let resp = client.list_repos(ListReposRequest {}).await?;
        Ok(resp.into_inner())
    }

    /// Create a new repository.
    pub async fn create_repo(
        &self,
        name: &str,
        description: &str,
    ) -> anyhow::Result<CreateRepoResponse> {
        let mut client = self.forge();
        let resp = client
            .create_repo(CreateRepoRequest {
                name: name.to_string(),
                description: description.to_string(),
            })
            .await?;
        Ok(resp.into_inner())
    }

    /// Update a repository (rename and/or change description).
    pub async fn update_repo(
        &self,
        name: &str,
        new_name: &str,
        description: &str,
    ) -> anyhow::Result<UpdateRepoResponse> {
        let mut client = self.forge();
        let resp = client
            .update_repo(UpdateRepoRequest {
                name: name.to_string(),
                new_name: new_name.to_string(),
                description: description.to_string(),
            })
            .await?;
        Ok(resp.into_inner())
    }

    /// Delete a repository.
    pub async fn delete_repo(&self, name: &str) -> anyhow::Result<DeleteRepoResponse> {
        let mut client = self.forge();
        let resp = client
            .delete_repo(DeleteRepoRequest {
                name: name.to_string(),
            })
            .await?;
        Ok(resp.into_inner())
    }

    /// List refs (branches) for a repository.
    pub async fn get_refs(&self, repo: &str) -> anyhow::Result<GetRefsResponse> {
        let mut client = self.forge();
        let resp = client
            .get_refs(GetRefsRequest {
                repo: repo.to_string(),
            })
            .await?;
        Ok(resp.into_inner())
    }

    /// List commits on a branch within a repository.
    pub async fn list_commits(
        &self,
        repo: &str,
        branch: &str,
        limit: i32,
        offset: i32,
    ) -> anyhow::Result<ListCommitsResponse> {
        let mut client = self.forge();
        let resp = client
            .list_commits(ListCommitsRequest {
                repo: repo.to_string(),
                branch: branch.to_string(),
                limit,
                offset,
            })
            .await?;
        Ok(resp.into_inner())
    }

    /// Get tree entries for a given commit and path within a repository.
    pub async fn get_tree_entries(
        &self,
        repo: &str,
        commit_hash: &str,
        path: &str,
    ) -> anyhow::Result<GetTreeEntriesResponse> {
        let mut client = self.forge();
        let resp = client
            .get_tree_entries(GetTreeEntriesRequest {
                repo: repo.to_string(),
                commit_hash: commit_hash.to_string(),
                path: path.to_string(),
            })
            .await?;
        Ok(resp.into_inner())
    }

    /// Get file content at a given commit and path within a repository.
    pub async fn get_file_content(
        &self,
        repo: &str,
        commit_hash: &str,
        path: &str,
    ) -> anyhow::Result<GetFileContentResponse> {
        let mut client = self.forge();
        let resp = client
            .get_file_content(GetFileContentRequest {
                repo: repo.to_string(),
                commit_hash: commit_hash.to_string(),
                path: path.to_string(),
            })
            .await?;
        Ok(resp.into_inner())
    }

    /// Get detailed commit info including changed files.
    pub async fn get_commit_detail(
        &self,
        repo: &str,
        commit_hash: &str,
    ) -> anyhow::Result<GetCommitDetailResponse> {
        let mut client = self.forge();
        let resp = client
            .get_commit_detail(GetCommitDetailRequest {
                repo: repo.to_string(),
                commit_hash: commit_hash.to_string(),
            })
            .await?;
        Ok(resp.into_inner())
    }

    /// Get server info (version, uptime, stats).
    pub async fn get_server_info(&self) -> anyhow::Result<GetServerInfoResponse> {
        let mut client = self.forge();
        let resp = client
            .get_server_info(GetServerInfoRequest {})
            .await?;
        Ok(resp.into_inner())
    }

    /// List file locks for a repository, optionally filtered by path prefix and/or owner.
    pub async fn list_locks(
        &self,
        repo: &str,
        path_prefix: &str,
        owner: &str,
    ) -> anyhow::Result<ListLocksResponse> {
        let mut client = self.forge();
        let resp = client
            .list_locks(ListLocksRequest {
                repo: repo.to_string(),
                path_prefix: path_prefix.to_string(),
                owner: owner.to_string(),
            })
            .await?;
        Ok(resp.into_inner())
    }

    /// Acquire a file lock in a repository.
    pub async fn acquire_lock(
        &self,
        repo: &str,
        path: &str,
        owner: &str,
        workspace_id: &str,
        reason: &str,
    ) -> anyhow::Result<LockResponse> {
        let mut client = self.forge();
        let resp = client
            .acquire_lock(LockRequest {
                repo: repo.to_string(),
                path: path.to_string(),
                owner: owner.to_string(),
                workspace_id: workspace_id.to_string(),
                reason: reason.to_string(),
            })
            .await?;
        Ok(resp.into_inner())
    }

    /// Release a file lock in a repository.
    pub async fn release_lock(
        &self,
        repo: &str,
        path: &str,
        owner: &str,
        force: bool,
    ) -> anyhow::Result<UnlockResponse> {
        let mut client = self.forge();
        let resp = client
            .release_lock(UnlockRequest {
                repo: repo.to_string(),
                path: path.to_string(),
                owner: owner.to_string(),
                force,
            })
            .await?;
        Ok(resp.into_inner())
    }

    // ── Actions ──

    pub async fn list_workflows(&self, repo: &str) -> anyhow::Result<ListWorkflowsResponse> {
        let mut client = self.forge();
        let resp = client.list_workflows(ListWorkflowsRequest { repo: repo.to_string() }).await?;
        Ok(resp.into_inner())
    }

    pub async fn create_workflow(&self, repo: &str, name: &str, yaml: &str) -> anyhow::Result<CreateWorkflowResponse> {
        let mut client = self.forge();
        let resp = client.create_workflow(CreateWorkflowRequest {
            repo: repo.to_string(), name: name.to_string(), yaml: yaml.to_string(),
        }).await?;
        Ok(resp.into_inner())
    }

    pub async fn update_workflow(&self, id: i64, name: &str, yaml: &str, enabled: bool) -> anyhow::Result<UpdateWorkflowResponse> {
        let mut client = self.forge();
        let resp = client.update_workflow(UpdateWorkflowRequest {
            id, name: name.to_string(), yaml: yaml.to_string(), enabled,
        }).await?;
        Ok(resp.into_inner())
    }

    pub async fn delete_workflow(&self, id: i64) -> anyhow::Result<DeleteWorkflowResponse> {
        let mut client = self.forge();
        let resp = client.delete_workflow(DeleteWorkflowRequest { id }).await?;
        Ok(resp.into_inner())
    }

    pub async fn trigger_workflow(&self, workflow_id: i64, ref_name: &str, triggered_by: &str) -> anyhow::Result<TriggerWorkflowResponse> {
        let mut client = self.forge();
        let resp = client.trigger_workflow(TriggerWorkflowRequest {
            workflow_id, ref_name: ref_name.to_string(), triggered_by: triggered_by.to_string(),
        }).await?;
        Ok(resp.into_inner())
    }

    pub async fn list_workflow_runs(&self, repo: &str, workflow_id: i64, limit: i32, offset: i32) -> anyhow::Result<ListWorkflowRunsResponse> {
        let mut client = self.forge();
        let resp = client.list_workflow_runs(ListWorkflowRunsRequest {
            repo: repo.to_string(), workflow_id, limit, offset,
        }).await?;
        Ok(resp.into_inner())
    }

    pub async fn get_workflow_run(&self, run_id: i64) -> anyhow::Result<GetWorkflowRunResponse> {
        let mut client = self.forge();
        let resp = client.get_workflow_run(GetWorkflowRunRequest { run_id }).await?;
        Ok(resp.into_inner())
    }

    pub async fn cancel_workflow_run(&self, run_id: i64) -> anyhow::Result<CancelWorkflowRunResponse> {
        let mut client = self.forge();
        let resp = client.cancel_workflow_run(CancelWorkflowRunRequest { run_id }).await?;
        Ok(resp.into_inner())
    }

    pub async fn list_artifacts(&self, run_id: i64) -> anyhow::Result<ListArtifactsResponse> {
        let mut client = self.forge();
        let resp = client.list_artifacts(ListArtifactsRequest { run_id }).await?;
        Ok(resp.into_inner())
    }

    pub async fn list_releases(&self, repo: &str) -> anyhow::Result<ListReleasesResponse> {
        let mut client = self.forge();
        let resp = client.list_releases(ListReleasesRequest { repo: repo.to_string() }).await?;
        Ok(resp.into_inner())
    }

    pub async fn get_release(&self, release_id: i64) -> anyhow::Result<GetReleaseResponse> {
        let mut client = self.forge();
        let resp = client.get_release(GetReleaseRequest { release_id }).await?;
        Ok(resp.into_inner())
    }

    // ── Issues ──

    pub async fn list_issues(&self, repo: &str, status: &str, limit: i32, offset: i32) -> anyhow::Result<ListIssuesResponse> {
        let mut client = self.forge();
        let resp = client.list_issues(ListIssuesRequest {
            repo: repo.to_string(), status: status.to_string(), limit, offset,
        }).await?;
        Ok(resp.into_inner())
    }

    pub async fn create_issue(&self, repo: &str, title: &str, body: &str, author: &str, labels: Vec<String>) -> anyhow::Result<CreateIssueResponse> {
        let mut client = self.forge();
        let resp = client.create_issue(CreateIssueRequest {
            repo: repo.to_string(), title: title.to_string(), body: body.to_string(),
            author: author.to_string(), labels,
        }).await?;
        Ok(resp.into_inner())
    }

    pub async fn update_issue(&self, id: i64, title: &str, body: &str, status: &str, labels: Vec<String>, assignee: &str) -> anyhow::Result<UpdateIssueResponse> {
        let mut client = self.forge();
        let resp = client.update_issue(UpdateIssueRequest {
            id, title: title.to_string(), body: body.to_string(),
            status: status.to_string(), labels, assignee: assignee.to_string(),
        }).await?;
        Ok(resp.into_inner())
    }

    pub async fn get_issue(&self, id: i64) -> anyhow::Result<GetIssueResponse> {
        let mut client = self.forge();
        let resp = client.get_issue(GetIssueRequest { id }).await?;
        Ok(resp.into_inner())
    }

    // ── Pull Requests ──

    pub async fn list_pull_requests(&self, repo: &str, status: &str, limit: i32, offset: i32) -> anyhow::Result<ListPullRequestsResponse> {
        let mut client = self.forge();
        let resp = client.list_pull_requests(ListPullRequestsRequest {
            repo: repo.to_string(), status: status.to_string(), limit, offset,
        }).await?;
        Ok(resp.into_inner())
    }

    pub async fn create_pull_request(&self, repo: &str, title: &str, body: &str, author: &str, source_branch: &str, target_branch: &str, labels: Vec<String>) -> anyhow::Result<CreatePullRequestResponse> {
        let mut client = self.forge();
        let resp = client.create_pull_request(CreatePullRequestRequest {
            repo: repo.to_string(), title: title.to_string(), body: body.to_string(),
            author: author.to_string(), source_branch: source_branch.to_string(),
            target_branch: target_branch.to_string(), labels,
        }).await?;
        Ok(resp.into_inner())
    }

    pub async fn update_pull_request(&self, id: i64, title: &str, body: &str, status: &str, labels: Vec<String>, assignee: &str) -> anyhow::Result<UpdatePullRequestResponse> {
        let mut client = self.forge();
        let resp = client.update_pull_request(UpdatePullRequestRequest {
            id, title: title.to_string(), body: body.to_string(),
            status: status.to_string(), labels, assignee: assignee.to_string(),
        }).await?;
        Ok(resp.into_inner())
    }

    pub async fn get_pull_request(&self, id: i64) -> anyhow::Result<GetPullRequestResponse> {
        let mut client = self.forge();
        let resp = client.get_pull_request(GetPullRequestRequest { id }).await?;
        Ok(resp.into_inner())
    }

    pub async fn merge_pull_request(&self, id: i64) -> anyhow::Result<MergePullRequestResponse> {
        let mut client = self.forge();
        let resp = client.merge_pull_request(MergePullRequestRequest { id }).await?;
        Ok(resp.into_inner())
    }
}
