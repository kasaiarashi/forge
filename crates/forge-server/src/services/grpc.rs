// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use std::sync::Arc;
use std::time::Instant;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use forge_core::hash::ForgeHash;
use forge_core::store::object_store::ObjectStore;
use forge_proto::forge::forge_service_server::ForgeService;
use forge_proto::forge::*;

use crate::auth::authorize::{
    require_authenticated, require_repo_admin, require_repo_read, require_repo_write,
};
use crate::auth::interceptor::caller_of;
use crate::auth::UserStore;
use crate::storage::db::MetadataDb;
use crate::storage::fs::FsStorage;

pub struct ForgeGrpcService {
    pub fs: Arc<FsStorage>,
    pub db: Arc<MetadataDb>,
    pub start_time: Instant,
    /// Channel to queue workflow runs for execution (Phase 3).
    pub workflow_engine: Option<tokio::sync::mpsc::Sender<i64>>,
    /// Auth/identity store. Used by every handler to check the caller's
    /// repo role and PAT scope before doing real work.
    pub user_store: Arc<dyn UserStore>,
}

/// Normalize a repo identifier into the canonical `<owner>/<name>` form
/// and validate it.
///
/// - `"alice/forge"` → returned as-is after validation.
/// - `"forge"` → if the caller is authenticated, returns `"<caller_username>/forge"`.
///   Anonymous callers cannot use the bare form (we have nothing to prepend).
/// - `""` → `InvalidArgument`.
///
/// The CLI's existing workspace config field `repo = "forge"` therefore keeps
/// working without flag changes — the server fills in the owner from the
/// authenticated PAT/session.
fn resolve_repo(repo: &str, caller: &crate::auth::Caller) -> Result<String, Status> {
    if repo.is_empty() {
        return Err(Status::invalid_argument("repo must not be empty"));
    }
    let full = if repo.contains('/') {
        repo.to_string()
    } else {
        match caller.username() {
            Some(u) => format!("{u}/{repo}"),
            None => {
                return Err(Status::unauthenticated(
                    "anonymous callers must use the full '<owner>/<name>' form",
                ));
            }
        }
    };
    super::validate::repo_name(&full)?;
    Ok(full)
}

impl ForgeGrpcService {
    /// Build an ObjectStore for a specific repo.
    fn object_store(&self, repo: &str) -> ObjectStore {
        let store = self.fs.repo_store(repo);
        ObjectStore::new(store.root().to_path_buf())
    }
}

#[tonic::async_trait]
impl ForgeService for ForgeGrpcService {
    type PullObjectsStream = ReceiverStream<Result<ObjectChunk, Status>>;

    async fn push_objects(
        &self,
        request: Request<Streaming<ObjectChunk>>,
    ) -> Result<Response<PushResponse>, Status> {
        let caller = caller_of(&request);
        let mut stream = request.into_inner();
        let mut received = Vec::new();
        // Buffer for reassembling multi-chunk objects.
        let mut current_buf: Vec<u8> = Vec::new();
        let mut current_hash: Option<Vec<u8>> = None;
        let mut store = None;

        while let Some(chunk) = stream
            .message()
            .await
            .map_err(|e| Status::internal(e.to_string()))?
        {
            // Read repo from the first chunk.
            if store.is_none() {
                let repo = resolve_repo(&chunk.repo, &caller)?;
                // Authz happens once, on the first chunk (after we know the repo).
                require_repo_write(&caller, &self.user_store, &repo)?;
                // Auto-register repo if it doesn't exist.
                self.db.create_repo(&repo, "")
                    .map_err(|e| Status::internal(format!("failed to register repo: {}", e)))?;
                store = Some(self.fs.repo_store(&repo));
            }

            if current_hash.as_ref() != Some(&chunk.hash) {
                // New object starting.
                current_buf.clear();
                current_hash = Some(chunk.hash.clone());
            }

            const MAX_OBJECT_SIZE: usize = 512 * 1024 * 1024; // 512 MiB per object
            if current_buf.len() + chunk.data.len() > MAX_OBJECT_SIZE {
                return Err(Status::resource_exhausted("object exceeds maximum size"));
            }
            current_buf.extend_from_slice(&chunk.data);

            if chunk.is_last {
                // Object complete — store it.
                let hash_bytes: [u8; 32] = chunk
                    .hash
                    .as_slice()
                    .try_into()
                    .map_err(|_| Status::invalid_argument("invalid hash length"))?;
                let forge_hash = ForgeHash::from_hex(&hex::encode(hash_bytes))
                    .map_err(|e| Status::internal(e.to_string()))?;

                let s = store.as_ref().ok_or_else(|| Status::internal("no repo specified in stream"))?;

                if chunk.object_type == 1 {
                    // Pre-compressed data — verify zstd magic bytes (0xFD2FB528).
                    if current_buf.len() < 4
                        || current_buf[0] != 0x28
                        || current_buf[1] != 0xB5
                        || current_buf[2] != 0x2F
                        || current_buf[3] != 0xFD
                    {
                        return Err(Status::data_loss(format!(
                            "invalid compressed data for {} (bad magic bytes)",
                            hex::encode(&hash_bytes)
                        )));
                    }
                    s.put_raw(&forge_hash, &current_buf)
                        .map_err(|e| Status::internal(e.to_string()))?;
                } else {
                    // Uncompressed data — compress and store.
                    s.put(&forge_hash, &current_buf)
                        .map_err(|e| Status::internal(e.to_string()))?;
                }

                received.push(chunk.hash.clone());
                current_buf.clear();
                current_hash = None;
            }
        }

        Ok(Response::new(PushResponse {
            received_hashes: received,
            error: String::new(),
        }))
    }

    async fn pull_objects(
        &self,
        request: Request<PullRequest>,
    ) -> Result<Response<Self::PullObjectsStream>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo = resolve_repo(&req.repo, &caller)?;
        // TODO(phase 6): pass real `public` flag from repos.visibility.
        require_repo_read(&caller, &self.user_store, &repo, self.db.is_repo_public(&repo))?;

        const MAX_PULL_HASHES: usize = 10_000;
        if req.want_hashes.len() > MAX_PULL_HASHES {
            return Err(Status::invalid_argument(format!(
                "too many hashes requested ({}, max {})", req.want_hashes.len(), MAX_PULL_HASHES
            )));
        }

        let store = self.fs.repo_store(&repo);

        let (tx, rx) = tokio::sync::mpsc::channel(32);

        tokio::spawn(async move {
            for hash_bytes in req.want_hashes {
                let hash_hex = hex::encode(&hash_bytes);
                let forge_hash = match ForgeHash::from_hex(&hash_hex) {
                    Ok(h) => h,
                    Err(_) => continue,
                };

                match store.get(&forge_hash) {
                    Ok(data) => {
                        // Send in chunks of 2MB to stay under gRPC message limits.
                        let chunk_size = 2 * 1024 * 1024;
                        let total = data.len() as u64;
                        let mut offset = 0usize;

                        while offset < data.len() {
                            let end = (offset + chunk_size).min(data.len());
                            let is_last = end == data.len();

                            let msg = ObjectChunk {
                                hash: hash_bytes.clone(),
                                object_type: 0,
                                total_size: total,
                                offset: offset as u64,
                                data: data[offset..end].to_vec(),
                                is_last,
                                repo: String::new(),
                            };

                            if tx.send(Ok(msg)).await.is_err() {
                                return;
                            }
                            offset = end;
                        }
                    }
                    Err(_) => {
                        // Object not found — skip silently.
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn has_objects(
        &self,
        request: Request<HasObjectsRequest>,
    ) -> Result<Response<HasObjectsResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;
        let store = self.fs.repo_store(repo);
        let mut has = Vec::with_capacity(req.hashes.len());

        for hash_bytes in &req.hashes {
            let hash_hex = hex::encode(hash_bytes);
            let exists = match ForgeHash::from_hex(&hash_hex) {
                Ok(h) => store.has(&h),
                Err(_) => false,
            };
            has.push(exists);
        }

        Ok(Response::new(HasObjectsResponse { has }))
    }

    async fn get_refs(
        &self,
        request: Request<GetRefsRequest>,
    ) -> Result<Response<GetRefsResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;

        let all_refs = self
            .db
            .get_all_refs(repo)
            .map_err(|e| Status::internal(e.to_string()))?;

        let mut refs = std::collections::HashMap::new();
        for (name, hash) in all_refs {
            refs.insert(name, hash);
        }

        Ok(Response::new(GetRefsResponse { refs }))
    }

    async fn update_ref(
        &self,
        request: Request<UpdateRefRequest>,
    ) -> Result<Response<UpdateRefResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        super::validate::ref_name(&req.ref_name)?;
        require_repo_write(&caller, &self.user_store, repo)?;

        // Auto-register repo if it doesn't exist (first push creates it).
        self.db.create_repo(repo, "")
            .map_err(|e| Status::internal(format!("failed to register repo: {}", e)))?;

        let success = self
            .db
            .update_ref(repo, &req.ref_name, &req.old_hash, &req.new_hash, req.force)
            .map_err(|e| Status::internal(e.to_string()))?;

        // Check push triggers on successful ref update.
        if success {
            if let Some(engine_tx) = &self.workflow_engine {
                crate::services::actions::trigger::check_push_triggers(
                    &self.db, engine_tx, repo, &req.ref_name, &req.new_hash,
                );
            }
        }

        Ok(Response::new(UpdateRefResponse {
            success,
            error: if success {
                String::new()
            } else {
                "ref has been updated by another client".into()
            },
        }))
    }

    async fn acquire_lock(
        &self,
        request: Request<LockRequest>,
    ) -> Result<Response<LockResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        super::validate::path(&req.path)?;
        require_repo_write(&caller, &self.user_store, repo)?;

        let result = self
            .db
            .acquire_lock(repo, &req.path, &req.owner, &req.workspace_id, &req.reason)
            .map_err(|e| Status::internal(e.to_string()))?;

        match result {
            Ok(()) => Ok(Response::new(LockResponse {
                granted: true,
                existing_lock: None,
            })),
            Err(lock) => Ok(Response::new(LockResponse {
                granted: false,
                existing_lock: Some(LockInfo {
                    path: lock.path,
                    owner: lock.owner,
                    workspace_id: lock.workspace_id,
                    created_at: lock.created_at,
                    reason: lock.reason,
                }),
            })),
        }
    }

    async fn release_lock(
        &self,
        request: Request<UnlockRequest>,
    ) -> Result<Response<UnlockResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        super::validate::path(&req.path)?;
        require_repo_write(&caller, &self.user_store, repo)?;

        // When force-unlocking, verify the caller provided an owner identity.
        // Force-unlock is an admin action; log it for audit trail.
        if req.force && !req.owner.is_empty() {
            tracing::warn!(
                repo = repo,
                path = req.path,
                owner = req.owner,
                "Force-unlock requested"
            );
        }

        let success = self
            .db
            .release_lock(repo, &req.path, &req.owner, req.force)
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(UnlockResponse {
            success,
            error: if success {
                String::new()
            } else {
                "lock not found or owned by another user".into()
            },
        }))
    }

    async fn list_locks(
        &self,
        request: Request<ListLocksRequest>,
    ) -> Result<Response<ListLocksResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;

        let locks = self
            .db
            .list_locks(repo, &req.path_prefix, &req.owner)
            .map_err(|e| Status::internal(e.to_string()))?;

        let lock_infos: Vec<LockInfo> = locks
            .into_iter()
            .map(|l| LockInfo {
                path: l.path,
                owner: l.owner,
                workspace_id: l.workspace_id,
                created_at: l.created_at,
                reason: l.reason,
            })
            .collect();

        Ok(Response::new(ListLocksResponse { locks: lock_infos }))
    }

    async fn verify_locks(
        &self,
        request: Request<VerifyLocksRequest>,
    ) -> Result<Response<VerifyLocksResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;

        // Get all locks for the requested paths.
        let all_locks = self
            .db
            .list_locks(repo, "", "")
            .map_err(|e| Status::internal(e.to_string()))?;

        let mut ours = Vec::new();
        let mut theirs = Vec::new();

        let requested_paths: std::collections::HashSet<&str> =
            req.paths.iter().map(|s| s.as_str()).collect();

        for lock in all_locks {
            if !requested_paths.is_empty() && !requested_paths.contains(lock.path.as_str()) {
                continue;
            }

            let info = LockInfo {
                path: lock.path,
                owner: lock.owner.clone(),
                workspace_id: lock.workspace_id,
                created_at: lock.created_at,
                reason: lock.reason,
            };

            if lock.owner == req.owner {
                ours.push(info);
            } else {
                theirs.push(info);
            }
        }

        Ok(Response::new(VerifyLocksResponse { ours, theirs }))
    }

    // ================================================================
    // Repository management RPCs
    // ================================================================

    async fn list_repos(
        &self,
        request: Request<ListReposRequest>,
    ) -> Result<Response<ListReposResponse>, Status> {
        let caller = caller_of(&request);
        require_authenticated(&caller)?;
        // TODO(phase 6): filter to repos the caller has at least read on
        // (or that are public). For now any logged-in user sees the full
        // list — read access on individual repos still gates clone/pull.
        let repos = self
            .db
            .list_repos()
            .map_err(|e| Status::internal(e.to_string()))?;

        let mut repo_infos = Vec::new();
        for r in repos {
            // Get branch info for this repo.
            let all_refs = self
                .db
                .get_all_refs(&r.name)
                .map_err(|e| Status::internal(e.to_string()))?;

            let branches: Vec<_> = all_refs
                .iter()
                .filter(|(name, _)| name.starts_with("refs/heads/"))
                .collect();
            let branch_count = branches.len() as i32;

            // Try to get last commit info from the default branch (main).
            let default_branch = "main".to_string();
            let mut last_commit_message = String::new();
            let mut last_commit_author = String::new();
            let mut last_commit_time = 0i64;

            let main_ref = format!("refs/heads/{}", default_branch);
            if let Ok(Some(tip_bytes)) = self.db.get_ref(&r.name, &main_ref) {
                if let Ok(tip) = ForgeHash::from_hex(&hex::encode(&tip_bytes)) {
                    let os = self.object_store(&r.name);
                    if let Ok(snap) = os.get_snapshot(&tip) {
                        last_commit_message = snap.message.clone();
                        last_commit_author = snap.author.name.clone();
                        last_commit_time = snap.timestamp.timestamp();
                    }
                }
            }

            repo_infos.push(RepoInfo {
                name: r.name,
                description: r.description,
                created_at: r.created_at,
                branch_count,
                default_branch,
                last_commit_message,
                last_commit_author,
                last_commit_time,
                visibility: r.visibility,
            });
        }

        Ok(Response::new(ListReposResponse { repos: repo_infos }))
    }

    async fn create_repo(
        &self,
        request: Request<CreateRepoRequest>,
    ) -> Result<Response<CreateRepoResponse>, Status> {
        let caller = caller_of(&request);
        // Any logged-in user can create repos in their own namespace.
        // Server admins can create in any namespace.
        let auth = crate::auth::authorize::require_authenticated(&caller)?;
        let req = request.into_inner();

        if req.name.is_empty() {
            return Ok(Response::new(CreateRepoResponse {
                success: false,
                error: "repo name cannot be empty".into(),
            }));
        }

        // Resolve `<owner>/<name>` (auto-prepends caller's username on bare names).
        let repo = match resolve_repo(&req.name, &caller) {
            Ok(r) => r,
            Err(s) => {
                return Ok(Response::new(CreateRepoResponse {
                    success: false,
                    error: s.message().to_string(),
                }));
            }
        };

        // Owner-half check: a non-admin user cannot create a repo in someone
        // else's namespace.
        let owner = repo.split('/').next().unwrap_or("");
        if owner != auth.username && !auth.is_server_admin {
            return Err(Status::permission_denied(format!(
                "cannot create '{repo}' in another user's namespace"
            )));
        }

        let created = self
            .db
            .create_repo(&repo, &req.description)
            .map_err(|e| Status::internal(e.to_string()))?;

        if !created {
            return Ok(Response::new(CreateRepoResponse {
                success: false,
                error: format!("repo '{repo}' already exists"),
            }));
        }

        // Ensure the repo's objects directory exists.
        let _store = self.fs.repo_store(&repo);

        Ok(Response::new(CreateRepoResponse {
            success: true,
            error: String::new(),
        }))
    }

    async fn update_repo(
        &self,
        request: Request<UpdateRepoRequest>,
    ) -> Result<Response<UpdateRepoResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();

        if req.name.is_empty() {
            return Ok(Response::new(UpdateRepoResponse {
                success: false,
                error: "repo name cannot be empty".into(),
            }));
        }
        // Resolve `<owner>/<name>` and authz the admin role on the resolved path.
        let repo = match resolve_repo(&req.name, &caller) {
            Ok(r) => r,
            Err(s) => {
                return Ok(Response::new(UpdateRepoResponse {
                    success: false,
                    error: s.message().to_string(),
                }));
            }
        };
        require_repo_admin(&caller, &self.user_store, &repo)?;

        // For renames, the new name must also be in the same namespace
        // (or no namespace, in which case we keep the original owner).
        let new_name = if req.new_name.is_empty() {
            String::new()
        } else if req.new_name.contains('/') {
            req.new_name.clone()
        } else {
            // bare name → keep the original owner
            let owner = repo.split('/').next().unwrap_or("");
            format!("{owner}/{}", req.new_name)
        };

        // Update the database record.
        match self.db.update_repo(&repo, &new_name, &req.description) {
            Ok(true) => {}
            Ok(false) => {
                return Ok(Response::new(UpdateRepoResponse {
                    success: false,
                    error: format!("repo '{repo}' not found"),
                }));
            }
            Err(e) => {
                return Ok(Response::new(UpdateRepoResponse {
                    success: false,
                    error: e.to_string(),
                }));
            }
        }

        // Apply visibility change if provided. Use the post-rename effective
        // name so it works alongside a rename in the same call.
        if !req.visibility.is_empty() {
            let effective = if new_name.is_empty() { repo.clone() } else { new_name.clone() };
            if let Err(e) = self.db.set_repo_visibility(&effective, &req.visibility) {
                return Ok(Response::new(UpdateRepoResponse {
                    success: false,
                    error: format!("visibility update failed: {e}"),
                }));
            }
        }

        // If renamed, also rename the filesystem directory.
        if !new_name.is_empty() && new_name != repo {
            if let Err(e) = self.fs.rename_repo(&repo, &new_name) {
                return Ok(Response::new(UpdateRepoResponse {
                    success: false,
                    error: format!("db updated but fs rename failed: {}", e),
                }));
            }
        }

        Ok(Response::new(UpdateRepoResponse {
            success: true,
            error: String::new(),
        }))
    }

    async fn delete_repo(
        &self,
        request: Request<DeleteRepoRequest>,
    ) -> Result<Response<DeleteRepoResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        if req.name.is_empty() {
            return Ok(Response::new(DeleteRepoResponse {
                success: false,
                error: "repo name cannot be empty".into(),
            }));
        }
        let repo = match resolve_repo(&req.name, &caller) {
            Ok(r) => r,
            Err(s) => {
                return Ok(Response::new(DeleteRepoResponse {
                    success: false,
                    error: s.message().to_string(),
                }));
            }
        };
        require_repo_admin(&caller, &self.user_store, &repo)?;

        // Delete from the database.
        let deleted = self
            .db
            .delete_repo(&repo)
            .map_err(|e| Status::internal(e.to_string()))?;

        if !deleted {
            return Ok(Response::new(DeleteRepoResponse {
                success: false,
                error: format!("repo '{repo}' not found"),
            }));
        }

        // Delete from the filesystem.
        if let Err(e) = self.fs.delete_repo(&repo) {
            return Ok(Response::new(DeleteRepoResponse {
                success: false,
                error: format!("db deleted but fs cleanup failed: {}", e),
            }));
        }

        Ok(Response::new(DeleteRepoResponse {
            success: true,
            error: String::new(),
        }))
    }

    // ================================================================
    // Browsing RPCs (for Web UI)
    // ================================================================

    async fn list_commits(
        &self,
        request: Request<ListCommitsRequest>,
    ) -> Result<Response<ListCommitsResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;
        let os = self.object_store(repo);

        let ref_name = format!("refs/heads/{}", if req.branch.is_empty() { "main" } else { &req.branch });
        let tip_bytes = self.db.get_ref(repo, &ref_name)
            .map_err(|e| Status::internal(e.to_string()))?;

        let tip = match tip_bytes {
            Some(b) => ForgeHash::from_hex(&hex::encode(&b))
                .map_err(|e| Status::internal(e.to_string()))?,
            None => return Ok(Response::new(ListCommitsResponse { commits: vec![], total: 0 })),
        };

        let limit = if req.limit == 0 { 50 } else { req.limit as usize };
        let offset = req.offset as usize;
        let mut commits = Vec::new();
        let mut current = tip;
        let mut skipped = 0usize;

        while !current.is_zero() && commits.len() < limit {
            let snap = match os.get_snapshot(&current) {
                Ok(s) => s,
                Err(_) => break,
            };

            if skipped < offset {
                skipped += 1;
            } else {
                commits.push(CommitInfo {
                    hash: current.to_hex(),
                    message: snap.message.clone(),
                    author_name: snap.author.name.clone(),
                    author_email: snap.author.email.clone(),
                    timestamp: snap.timestamp.timestamp(),
                    parent_hashes: snap.parents.iter().map(|p| p.to_hex()).collect(),
                });
            }

            current = snap.parents.first().copied().unwrap_or(ForgeHash::ZERO);
        }

        let total = (skipped + commits.len()) as i32;
        Ok(Response::new(ListCommitsResponse { commits, total }))
    }

    async fn get_tree_entries(
        &self,
        request: Request<GetTreeEntriesRequest>,
    ) -> Result<Response<GetTreeEntriesResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;
        let os = self.object_store(repo);

        let commit_hash = ForgeHash::from_hex(&req.commit_hash)
            .map_err(|e| Status::internal(e.to_string()))?;
        let snap = os.get_snapshot(&commit_hash)
            .map_err(|e| Status::internal(e.to_string()))?;

        // Navigate to the requested path within the tree.
        let mut tree_hash = snap.tree;

        if !req.path.is_empty() {
            for component in req.path.split('/').filter(|c| !c.is_empty()) {
                let tree = os.get_tree(&tree_hash)
                    .map_err(|e| Status::internal(e.to_string()))?;
                let entry = tree.entries.iter()
                    .find(|e| e.name == component)
                    .ok_or_else(|| Status::not_found(format!("Path not found: {}", req.path)))?;
                if entry.kind != forge_core::object::tree::EntryKind::Directory {
                    return Err(Status::invalid_argument(format!("{} is not a directory", component)));
                }
                tree_hash = entry.hash;
            }
        }

        let tree = os.get_tree(&tree_hash)
            .map_err(|e| Status::internal(e.to_string()))?;

        let mut entries: Vec<TreeEntryInfo> = tree.entries.iter().map(|e| {
            // For .uasset/.umap files, try a quick header parse for the asset class.
            let asset_class = if forge_core::uasset::is_uasset_path(&e.name)
                && e.kind == forge_core::object::tree::EntryKind::File
            {
                os.get_blob_data(&e.hash)
                    .ok()
                    .and_then(|data| forge_core::uasset::parse_uasset(&data))
                    .map(|m| m.asset_class)
                    .unwrap_or_default()
            } else {
                String::new()
            };

            TreeEntryInfo {
                name: e.name.clone(),
                kind: match e.kind {
                    forge_core::object::tree::EntryKind::File => "file".into(),
                    forge_core::object::tree::EntryKind::Directory => "directory".into(),
                    forge_core::object::tree::EntryKind::Symlink => "symlink".into(),
                },
                hash: e.hash.short(),
                size: e.size,
                asset_class,
            }
        }).collect();

        // Sort: directories first, then files, alphabetically.
        entries.sort_by(|a, b| {
            let a_dir = a.kind == "directory";
            let b_dir = b.kind == "directory";
            b_dir.cmp(&a_dir).then(a.name.cmp(&b.name))
        });

        Ok(Response::new(GetTreeEntriesResponse {
            entries,
            commit_hash: req.commit_hash,
            path: req.path,
        }))
    }

    async fn get_file_content(
        &self,
        request: Request<GetFileContentRequest>,
    ) -> Result<Response<GetFileContentResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;
        let os = self.object_store(repo);

        let commit_hash = ForgeHash::from_hex(&req.commit_hash)
            .map_err(|e| Status::internal(e.to_string()))?;
        let snap = os.get_snapshot(&commit_hash)
            .map_err(|e| Status::internal(e.to_string()))?;

        // Navigate to the file.
        let mut tree_hash = snap.tree;
        let parts: Vec<&str> = req.path.split('/').filter(|c| !c.is_empty()).collect();
        let (dir_parts, file_name) = parts.split_at(parts.len().saturating_sub(1));

        for component in dir_parts {
            let tree = os.get_tree(&tree_hash)
                .map_err(|e| Status::internal(e.to_string()))?;
            let entry = tree.entries.iter()
                .find(|e| e.name == *component)
                .ok_or_else(|| Status::not_found(format!("Path not found: {}", req.path)))?;
            tree_hash = entry.hash;
        }

        let tree = os.get_tree(&tree_hash)
            .map_err(|e| Status::internal(e.to_string()))?;
        let file_entry = tree.entries.iter()
            .find(|e| Some(e.name.as_str()) == file_name.first().copied())
            .ok_or_else(|| Status::not_found(format!("File not found: {}", req.path)))?;

        // Get the file content.
        let content = os.get_blob_data(&file_entry.hash)
            .map_err(|e| Status::internal(e.to_string()))?;

        let is_binary = content.iter().take(8192).any(|&b| b == 0);
        let size = content.len() as u64;

        // Parse UE asset metadata on-demand for .uasset/.umap files.
        let asset_metadata = if forge_core::uasset::is_uasset_path(&req.path) {
            forge_core::uasset::parse_uasset(&content).map(|m| AssetMetadata {
                asset_class: m.asset_class,
                engine_version: m.engine_version,
                package_flags: m.package_flags,
                dependencies: m.dependencies,
            })
        } else {
            None
        };

        Ok(Response::new(GetFileContentResponse {
            content,
            size,
            is_binary,
            hash: file_entry.hash.short(),
            asset_metadata,
        }))
    }

    async fn get_commit_detail(
        &self,
        request: Request<GetCommitDetailRequest>,
    ) -> Result<Response<GetCommitDetailResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;
        let os = self.object_store(repo);

        let commit_hash = ForgeHash::from_hex(&req.commit_hash)
            .map_err(|e| Status::internal(e.to_string()))?;
        let snap = os.get_snapshot(&commit_hash)
            .map_err(|e| Status::internal(e.to_string()))?;

        let commit = CommitInfo {
            hash: commit_hash.to_hex(),
            message: snap.message.clone(),
            author_name: snap.author.name.clone(),
            author_email: snap.author.email.clone(),
            timestamp: snap.timestamp.timestamp(),
            parent_hashes: snap.parents.iter().map(|p| p.to_hex()).collect(),
        };

        // Diff against parent to find changed files.
        let changes = if let Some(parent_hash) = snap.parents.first() {
            if let Ok(parent_snap) = os.get_snapshot(parent_hash) {
                let get_tree = |h: &ForgeHash| os.get_tree(h).ok();
                let old_map = forge_core::diff::flatten_tree(
                    &os.get_tree(&parent_snap.tree).unwrap_or_default(),
                    "",
                    &get_tree,
                );
                let new_map = forge_core::diff::flatten_tree(
                    &os.get_tree(&snap.tree).unwrap_or_default(),
                    "",
                    &get_tree,
                );
                forge_core::diff::diff_maps(&old_map, &new_map)
                    .into_iter()
                    .map(|d| match d {
                        forge_core::diff::DiffEntry::Added { path, size, .. } => DiffEntry {
                            path, change_type: "added".into(), old_size: 0, new_size: size,
                        },
                        forge_core::diff::DiffEntry::Deleted { path, size, .. } => DiffEntry {
                            path, change_type: "deleted".into(), old_size: size, new_size: 0,
                        },
                        forge_core::diff::DiffEntry::Modified { path, old_size, new_size, .. } => DiffEntry {
                            path, change_type: "modified".into(), old_size, new_size,
                        },
                    })
                    .collect()
            } else {
                vec![]
            }
        } else {
            // Initial commit: all files are "added".
            let get_tree = |h: &ForgeHash| os.get_tree(h).ok();
            let tree = os.get_tree(&snap.tree).unwrap_or_default();
            let map = forge_core::diff::flatten_tree(&tree, "", &get_tree);
            map.into_iter()
                .map(|(path, (_, size))| DiffEntry {
                    path, change_type: "added".into(), old_size: 0, new_size: size,
                })
                .collect()
        };

        Ok(Response::new(GetCommitDetailResponse {
            commit: Some(commit),
            changes,
        }))
    }

    async fn get_server_info(
        &self,
        request: Request<GetServerInfoRequest>,
    ) -> Result<Response<GetServerInfoResponse>, Status> {
        let caller = caller_of(&request);
        require_authenticated(&caller)?;
        let uptime = self.start_time.elapsed().as_secs() as i64;

        let repos = self.db.list_repos()
            .map_err(|e| Status::internal(e.to_string()))?;
        let repo_names: Vec<String> = repos.iter().map(|r| r.name.clone()).collect();

        // Count total active locks across all repos (sum per-repo).
        let mut total_locks = 0i32;
        for r in &repos {
            let locks = self.db.list_locks(&r.name, "", "")
                .map_err(|e| Status::internal(e.to_string()))?;
            total_locks += locks.len() as i32;
        }

        Ok(Response::new(GetServerInfoResponse {
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs: uptime,
            total_objects: 0, // TODO: count objects
            total_size_bytes: 0,
            repos: repo_names,
            active_locks: total_locks,
        }))
    }

    // ================================================================
    // Actions — Workflows
    // ================================================================

    async fn list_workflows(
        &self,
        request: Request<ListWorkflowsRequest>,
    ) -> Result<Response<ListWorkflowsResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;
        let workflows = self.db.list_workflows(repo)
            .map_err(|e| Status::internal(e.to_string()))?;
        let infos = workflows.into_iter().map(|w| WorkflowInfo {
            id: w.id, repo: w.repo, name: w.name, yaml: w.yaml,
            enabled: w.enabled, created_at: w.created_at, updated_at: w.updated_at,
        }).collect();
        Ok(Response::new(ListWorkflowsResponse { workflows: infos }))
    }

    async fn create_workflow(
        &self,
        request: Request<CreateWorkflowRequest>,
    ) -> Result<Response<CreateWorkflowResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_admin(&caller, &self.user_store, repo)?;
        // Validate YAML before saving.
        if let Err(e) = crate::services::actions::yaml::WorkflowDef::parse(&req.yaml) {
            return Ok(Response::new(CreateWorkflowResponse {
                success: false, error: format!("Invalid workflow YAML: {e}"), id: 0,
            }));
        }
        match self.db.create_workflow(repo, &req.name, &req.yaml) {
            Ok(id) => Ok(Response::new(CreateWorkflowResponse { success: true, error: String::new(), id })),
            Err(e) => Ok(Response::new(CreateWorkflowResponse { success: false, error: e.to_string(), id: 0 })),
        }
    }

    async fn update_workflow(
        &self,
        request: Request<UpdateWorkflowRequest>,
    ) -> Result<Response<UpdateWorkflowResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        // Look up the workflow's repo so we can authz against it.
        let workflow = self.db.get_workflow(req.id)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Workflow not found"))?;
        require_repo_admin(&caller, &self.user_store, &workflow.repo)?;
        if !req.yaml.is_empty() {
            if let Err(e) = crate::services::actions::yaml::WorkflowDef::parse(&req.yaml) {
                return Ok(Response::new(UpdateWorkflowResponse {
                    success: false, error: format!("Invalid workflow YAML: {e}"),
                }));
            }
        }
        match self.db.update_workflow(req.id, &req.name, &req.yaml, req.enabled) {
            Ok(true) => Ok(Response::new(UpdateWorkflowResponse { success: true, error: String::new() })),
            Ok(false) => Ok(Response::new(UpdateWorkflowResponse { success: false, error: "Workflow not found".into() })),
            Err(e) => Ok(Response::new(UpdateWorkflowResponse { success: false, error: e.to_string() })),
        }
    }

    async fn delete_workflow(
        &self,
        request: Request<DeleteWorkflowRequest>,
    ) -> Result<Response<DeleteWorkflowResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let workflow = self.db.get_workflow(req.id)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Workflow not found"))?;
        require_repo_admin(&caller, &self.user_store, &workflow.repo)?;
        match self.db.delete_workflow(req.id) {
            Ok(true) => Ok(Response::new(DeleteWorkflowResponse { success: true, error: String::new() })),
            Ok(false) => Ok(Response::new(DeleteWorkflowResponse { success: false, error: "Workflow not found".into() })),
            Err(e) => Ok(Response::new(DeleteWorkflowResponse { success: false, error: e.to_string() })),
        }
    }

    // ================================================================
    // Actions — Runs
    // ================================================================

    async fn trigger_workflow(
        &self,
        request: Request<TriggerWorkflowRequest>,
    ) -> Result<Response<TriggerWorkflowResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let workflow = self.db.get_workflow(req.workflow_id)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Workflow not found"))?;
        require_repo_write(&caller, &self.user_store, &workflow.repo)?;
        if !workflow.enabled {
            return Ok(Response::new(TriggerWorkflowResponse {
                success: false, error: "Workflow is disabled".into(), run_id: 0,
            }));
        }
        // Check if manual trigger is allowed by the workflow definition.
        if let Ok(def) = crate::services::actions::yaml::WorkflowDef::parse(&workflow.yaml) {
            if !def.allows_manual() {
                return Ok(Response::new(TriggerWorkflowResponse {
                    success: false, error: "Manual trigger is not enabled for this workflow".into(), run_id: 0,
                }));
            }
        }
        // Resolve commit hash from the ref.
        let ref_name = if req.ref_name.is_empty() { "refs/heads/main".to_string() } else { req.ref_name };
        let commit_hash = self.db.get_ref(&workflow.repo, &ref_name)
            .map_err(|e| Status::internal(e.to_string()))?
            .map(|h| hex::encode(&h))
            .unwrap_or_default();

        let run_id = self.db.create_run(
            &workflow.repo, workflow.id, "manual", &ref_name, &commit_hash, &req.triggered_by,
        ).map_err(|e| Status::internal(e.to_string()))?;

        // Queue the run for execution (engine integration in Phase 3).
        if let Some(engine) = &self.workflow_engine {
            let _ = engine.send(run_id);
        }

        Ok(Response::new(TriggerWorkflowResponse { success: true, error: String::new(), run_id }))
    }

    async fn list_workflow_runs(
        &self,
        request: Request<ListWorkflowRunsRequest>,
    ) -> Result<Response<ListWorkflowRunsResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;
        let (runs, total) = self.db.list_runs(repo, req.workflow_id, req.limit, req.offset)
            .map_err(|e| Status::internal(e.to_string()))?;
        let infos = runs.into_iter().map(|r| WorkflowRunInfo {
            id: r.id, repo: r.repo, workflow_id: r.workflow_id,
            workflow_name: r.workflow_name, trigger: r.trigger,
            trigger_ref: r.trigger_ref, commit_hash: r.commit_hash,
            status: r.status, started_at: r.started_at.unwrap_or(0),
            finished_at: r.finished_at.unwrap_or(0), created_at: r.created_at,
            triggered_by: r.triggered_by,
        }).collect();
        Ok(Response::new(ListWorkflowRunsResponse { runs: infos, total }))
    }

    async fn get_workflow_run(
        &self,
        request: Request<GetWorkflowRunRequest>,
    ) -> Result<Response<GetWorkflowRunResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let run = self.db.get_run(req.run_id)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Run not found"))?;
        require_repo_read(&caller, &self.user_store, &run.repo, self.db.is_repo_public(&run.repo))?;
        let steps = self.db.list_steps(req.run_id)
            .map_err(|e| Status::internal(e.to_string()))?;
        let artifacts_list = self.db.list_artifacts(req.run_id)
            .map_err(|e| Status::internal(e.to_string()))?;

        let run_info = WorkflowRunInfo {
            id: run.id, repo: run.repo, workflow_id: run.workflow_id,
            workflow_name: run.workflow_name, trigger: run.trigger,
            trigger_ref: run.trigger_ref, commit_hash: run.commit_hash,
            status: run.status, started_at: run.started_at.unwrap_or(0),
            finished_at: run.finished_at.unwrap_or(0), created_at: run.created_at,
            triggered_by: run.triggered_by,
        };
        let step_infos = steps.into_iter().map(|s| StepInfo {
            id: s.id, job_name: s.job_name, step_index: s.step_index,
            name: s.name, status: s.status, exit_code: s.exit_code.unwrap_or(-1),
            log: s.log, started_at: s.started_at.unwrap_or(0),
            finished_at: s.finished_at.unwrap_or(0),
        }).collect();
        let artifact_infos = artifacts_list.into_iter().map(|a| ArtifactInfo {
            id: a.id, run_id: a.run_id, name: a.name,
            size_bytes: a.size_bytes, created_at: a.created_at,
        }).collect();

        Ok(Response::new(GetWorkflowRunResponse {
            run: Some(run_info), steps: step_infos, artifacts: artifact_infos,
        }))
    }

    async fn cancel_workflow_run(
        &self,
        request: Request<CancelWorkflowRunRequest>,
    ) -> Result<Response<CancelWorkflowRunResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let run = self.db.get_run(req.run_id)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Run not found"))?;
        require_repo_write(&caller, &self.user_store, &run.repo)?;
        if run.status != "queued" && run.status != "running" {
            return Ok(Response::new(CancelWorkflowRunResponse {
                success: false, error: format!("Cannot cancel run in '{}' state", run.status),
            }));
        }
        self.db.update_run_status(req.run_id, "cancelled")
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(CancelWorkflowRunResponse { success: true, error: String::new() }))
    }

    // ================================================================
    // Actions — Artifacts & Releases
    // ================================================================

    async fn list_artifacts(
        &self,
        request: Request<ListArtifactsRequest>,
    ) -> Result<Response<ListArtifactsResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        // Look up the run so we know which repo this artifact list belongs to.
        let run = self.db.get_run(req.run_id)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Run not found"))?;
        require_repo_read(&caller, &self.user_store, &run.repo, self.db.is_repo_public(&run.repo))?;
        let artifacts = self.db.list_artifacts(req.run_id)
            .map_err(|e| Status::internal(e.to_string()))?;
        let infos = artifacts.into_iter().map(|a| ArtifactInfo {
            id: a.id, run_id: a.run_id, name: a.name,
            size_bytes: a.size_bytes, created_at: a.created_at,
        }).collect();
        Ok(Response::new(ListArtifactsResponse { artifacts: infos }))
    }

    async fn list_releases(
        &self,
        request: Request<ListReleasesRequest>,
    ) -> Result<Response<ListReleasesResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;
        let releases = self.db.list_releases(repo)
            .map_err(|e| Status::internal(e.to_string()))?;
        let mut infos = Vec::new();
        for r in releases {
            let artifact_ids = self.db.get_release_artifact_ids(r.id)
                .map_err(|e| Status::internal(e.to_string()))?;
            let mut artifacts = Vec::new();
            for aid in artifact_ids {
                if let Ok(Some(a)) = self.db.get_artifact(aid) {
                    artifacts.push(ArtifactInfo {
                        id: a.id, run_id: a.run_id, name: a.name,
                        size_bytes: a.size_bytes, created_at: a.created_at,
                    });
                }
            }
            infos.push(ReleaseInfo {
                id: r.id, repo: r.repo, tag: r.tag, name: r.name,
                run_id: r.run_id.unwrap_or(0), created_at: r.created_at, artifacts,
            });
        }
        Ok(Response::new(ListReleasesResponse { releases: infos }))
    }

    async fn get_release(
        &self,
        request: Request<GetReleaseRequest>,
    ) -> Result<Response<GetReleaseResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let r = self.db.get_release(req.release_id)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Release not found"))?;
        require_repo_read(&caller, &self.user_store, &r.repo, self.db.is_repo_public(&r.repo))?;
        let artifact_ids = self.db.get_release_artifact_ids(r.id)
            .map_err(|e| Status::internal(e.to_string()))?;
        let mut artifacts = Vec::new();
        for aid in artifact_ids {
            if let Ok(Some(a)) = self.db.get_artifact(aid) {
                artifacts.push(ArtifactInfo {
                    id: a.id, run_id: a.run_id, name: a.name,
                    size_bytes: a.size_bytes, created_at: a.created_at,
                });
            }
        }
        Ok(Response::new(GetReleaseResponse {
            release: Some(ReleaseInfo {
                id: r.id, repo: r.repo, tag: r.tag, name: r.name,
                run_id: r.run_id.unwrap_or(0), created_at: r.created_at, artifacts,
            }),
        }))
    }

    // ── Issues ──

    async fn list_issues(
        &self,
        request: Request<ListIssuesRequest>,
    ) -> Result<Response<ListIssuesResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;
        let (issues, total, open_count, closed_count) = self.db
            .list_issues(repo, &req.status, req.limit, req.offset)
            .map_err(|e| Status::internal(e.to_string()))?;

        let infos: Vec<IssueInfo> = issues.into_iter().map(|i| {
            let labels = if i.labels.is_empty() { vec![] } else {
                i.labels.split(',').map(|s| s.trim().to_string()).collect()
            };
            IssueInfo {
                id: i.id, repo: i.repo, title: i.title, body: i.body,
                author: i.author, status: i.status, labels,
                created_at: i.created_at, updated_at: i.updated_at,
                comment_count: i.comment_count, assignee: i.assignee,
            }
        }).collect();

        Ok(Response::new(ListIssuesResponse { issues: infos, total, open_count, closed_count }))
    }

    async fn create_issue(
        &self,
        request: Request<CreateIssueRequest>,
    ) -> Result<Response<CreateIssueResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_write(&caller, &self.user_store, repo)?;
        let labels = req.labels.join(",");
        let id = self.db.create_issue(repo, &req.title, &req.body, &req.author, &labels)
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(CreateIssueResponse { success: true, error: String::new(), id }))
    }

    async fn update_issue(
        &self,
        request: Request<UpdateIssueRequest>,
    ) -> Result<Response<UpdateIssueResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        // Look up the issue's repo before mutating.
        let issue = self.db.get_issue(req.id)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Issue not found"))?;
        require_repo_write(&caller, &self.user_store, &issue.repo)?;
        let labels = req.labels.join(",");
        let ok = self.db.update_issue(req.id, &req.title, &req.body, &req.status, &labels, &req.assignee)
            .map_err(|e| Status::internal(e.to_string()))?;
        if !ok {
            return Ok(Response::new(UpdateIssueResponse { success: false, error: "Issue not found".into() }));
        }
        Ok(Response::new(UpdateIssueResponse { success: true, error: String::new() }))
    }

    // ── Pull Requests ──

    async fn list_pull_requests(
        &self,
        request: Request<ListPullRequestsRequest>,
    ) -> Result<Response<ListPullRequestsResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_read(&caller, &self.user_store, repo, self.db.is_repo_public(repo))?;
        let (prs, total, open_count, closed_count) = self.db
            .list_pull_requests(repo, &req.status, req.limit, req.offset)
            .map_err(|e| Status::internal(e.to_string()))?;

        let infos: Vec<PullRequestInfo> = prs.into_iter().map(|p| {
            let labels = if p.labels.is_empty() { vec![] } else {
                p.labels.split(',').map(|s| s.trim().to_string()).collect()
            };
            PullRequestInfo {
                id: p.id, repo: p.repo, title: p.title, body: p.body,
                author: p.author, status: p.status,
                source_branch: p.source_branch, target_branch: p.target_branch,
                labels, created_at: p.created_at, updated_at: p.updated_at,
                comment_count: p.comment_count, assignee: p.assignee,
            }
        }).collect();

        Ok(Response::new(ListPullRequestsResponse { pull_requests: infos, total, open_count, closed_count }))
    }

    async fn create_pull_request(
        &self,
        request: Request<CreatePullRequestRequest>,
    ) -> Result<Response<CreatePullRequestResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo_full = resolve_repo(&req.repo, &caller)?;
        let repo = repo_full.as_str();
        require_repo_write(&caller, &self.user_store, repo)?;
        let labels = req.labels.join(",");
        let id = self.db.create_pull_request(
            repo, &req.title, &req.body, &req.author,
            &req.source_branch, &req.target_branch, &labels,
        ).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(CreatePullRequestResponse { success: true, error: String::new(), id }))
    }

    async fn update_pull_request(
        &self,
        request: Request<UpdatePullRequestRequest>,
    ) -> Result<Response<UpdatePullRequestResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let pr = self.db.get_pull_request(req.id)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Pull request not found"))?;
        require_repo_write(&caller, &self.user_store, &pr.repo)?;
        let labels = req.labels.join(",");
        let ok = self.db.update_pull_request(req.id, &req.title, &req.body, &req.status, &labels, &req.assignee)
            .map_err(|e| Status::internal(e.to_string()))?;
        if !ok {
            return Ok(Response::new(UpdatePullRequestResponse { success: false, error: "Pull request not found".into() }));
        }
        Ok(Response::new(UpdatePullRequestResponse { success: true, error: String::new() }))
    }

    // ── Merge Pull Request ──

    async fn merge_pull_request(
        &self,
        request: Request<MergePullRequestRequest>,
    ) -> Result<Response<MergePullRequestResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();

        // Get the PR to find source/target branches
        let pr = self.db.get_pull_request(req.id)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Pull request not found"))?;
        require_repo_write(&caller, &self.user_store, &pr.repo)?;

        if pr.status != "open" {
            return Ok(Response::new(MergePullRequestResponse {
                success: false,
                error: format!("Pull request is already {}", pr.status),
            }));
        }

        // Get the source branch HEAD hash
        let source_ref = format!("refs/heads/{}", pr.source_branch);
        let source_hash = self.db.get_ref(&pr.repo, &source_ref)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found(format!("Source branch '{}' not found", pr.source_branch)))?;

        // Get the target branch HEAD hash
        let target_ref = format!("refs/heads/{}", pr.target_branch);
        let target_hash = self.db.get_ref(&pr.repo, &target_ref)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found(format!("Target branch '{}' not found", pr.target_branch)))?;

        // Fast-forward merge: CAS-update target branch to point to source
        // branch's HEAD. force = false: a merge that races with a direct
        // push to the target branch should fail and the user can retry.
        let updated = self.db.update_ref(&pr.repo, &target_ref, &target_hash, &source_hash, false)
            .map_err(|e| Status::internal(e.to_string()))?;

        if !updated {
            return Ok(Response::new(MergePullRequestResponse {
                success: false,
                error: "Failed to update target branch ref (concurrent modification?)".into(),
            }));
        }

        // Mark PR as merged
        self.db.update_pull_request(req.id, "", "", "merged", "", "")
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(MergePullRequestResponse { success: true, error: String::new() }))
    }

    // ── Single item getters ──

    async fn get_issue(
        &self,
        request: Request<GetIssueRequest>,
    ) -> Result<Response<GetIssueResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let issue = self.db.get_issue(req.id)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Issue not found"))?;
        require_repo_read(&caller, &self.user_store, &issue.repo, self.db.is_repo_public(&issue.repo))?;

        let labels = if issue.labels.is_empty() { vec![] } else {
            issue.labels.split(',').map(|s| s.trim().to_string()).collect()
        };
        Ok(Response::new(GetIssueResponse {
            issue: Some(IssueInfo {
                id: issue.id, repo: issue.repo, title: issue.title, body: issue.body,
                author: issue.author, status: issue.status, labels,
                created_at: issue.created_at, updated_at: issue.updated_at,
                comment_count: issue.comment_count, assignee: issue.assignee,
            }),
        }))
    }

    async fn get_pull_request(
        &self,
        request: Request<GetPullRequestRequest>,
    ) -> Result<Response<GetPullRequestResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let pr = self.db.get_pull_request(req.id)
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found("Pull request not found"))?;
        require_repo_read(&caller, &self.user_store, &pr.repo, self.db.is_repo_public(&pr.repo))?;

        let labels = if pr.labels.is_empty() { vec![] } else {
            pr.labels.split(',').map(|s| s.trim().to_string()).collect()
        };
        Ok(Response::new(GetPullRequestResponse {
            pull_request: Some(PullRequestInfo {
                id: pr.id, repo: pr.repo, title: pr.title, body: pr.body,
                author: pr.author, status: pr.status,
                source_branch: pr.source_branch, target_branch: pr.target_branch,
                labels, created_at: pr.created_at, updated_at: pr.updated_at,
                comment_count: pr.comment_count, assignee: pr.assignee,
            }),
        }))
    }
}
