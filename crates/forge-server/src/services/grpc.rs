// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use forge_core::hash::ForgeHash;
use forge_core::store::object_store::ObjectStore;
use forge_proto::forge::forge_service_server::ForgeService;
use forge_proto::forge::*;

use crate::audit;
use crate::auth::authorize::{
    require_authenticated, require_repo_admin, require_repo_read, require_repo_write,
};
use crate::auth::interceptor::caller_of;
use crate::auth::UserStore;
use crate::storage::db::MetadataDb;
use crate::storage::fs::FsStorage;

/// Log the raw error server-side and return a generic `Status::internal`.
/// Used to avoid leaking internal error messages (SQL schema, filesystem
/// paths, etc) to remote callers. The `label` is a short static string so
/// log grep still works.
fn internal_err<E: std::fmt::Display>(label: &'static str, err: E) -> Status {
    tracing::error!(op = label, error = %err, "internal error");
    Status::internal("internal server error")
}

pub struct ForgeGrpcService {
    pub fs: Arc<FsStorage>,
    pub db: Arc<MetadataDb>,
    pub start_time: Instant,
    /// Channel to queue workflow runs for execution (Phase 3).
    pub workflow_engine: Option<tokio::sync::mpsc::Sender<i64>>,
    /// Auth/identity store. Used by every handler to check the caller's
    /// repo role and PAT scope before doing real work.
    pub user_store: Arc<dyn UserStore>,
    /// Secret backend. Write-only through RPCs — values flow outward only
    /// to the run executor, never back to clients (no Read RPC exists).
    pub secrets: Arc<dyn crate::services::secrets::SecretBackend>,
    /// Artifact backend. Streams binary content without materialising whole
    /// blobs in memory.
    pub artifacts: Arc<dyn crate::services::artifacts::ArtifactStore>,
    /// Master key, used to sign short-lived artifact download URLs for the
    /// web UI. Held as raw bytes so the signer module stays dep-free.
    pub artifact_signer_key: [u8; 32],
    /// Live step-log broadcast hub. Engine/agents publish per-run chunks;
    /// `StreamStepLogs` subscribers tail them.
    pub log_hub: Arc<crate::services::logs::LogHub>,
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

/// Return true if `ancestor` is reachable from `descendant` via parent links
/// (or equal to it). Walks the snapshot DAG breadth-first. Bounded to avoid
/// pathological repos; the cap is far above any realistic divergence depth.
fn is_ancestor_or_equal(os: &ObjectStore, ancestor: &ForgeHash, descendant: &ForgeHash) -> bool {
    if ancestor == descendant {
        return true;
    }
    if ancestor.is_zero() {
        return true;
    }
    const MAX_WALK: usize = 100_000;
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![*descendant];
    let mut visited = 0usize;
    while let Some(cur) = stack.pop() {
        if cur.is_zero() || !seen.insert(cur) {
            continue;
        }
        visited += 1;
        if visited > MAX_WALK {
            tracing::warn!("is_ancestor_or_equal: walk cap hit, treating as non-ancestor");
            return false;
        }
        let snap = match os.get_snapshot(&cur) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for p in &snap.parents {
            if p == ancestor {
                return true;
            }
            stack.push(*p);
        }
    }
    false
}

#[tonic::async_trait]
impl ForgeService for ForgeGrpcService {
    type PullObjectsStream = ReceiverStream<Result<ObjectChunk, Status>>;
    type DownloadArtifactStream = Pin<Box<dyn futures::Stream<Item = Result<ArtifactChunk, Status>> + Send>>;
    type StreamStepLogsStream = Pin<Box<dyn futures::Stream<Item = Result<StepLogChunk, Status>> + Send>>;

    async fn push_objects(
        &self,
        request: Request<Streaming<ObjectChunk>>,
    ) -> Result<Response<PushResponse>, Status> {
        let caller = caller_of(&request);
        let mut stream = request.into_inner();
        let mut received: Vec<Vec<u8>> = Vec::new();
        // Buffer for reassembling multi-chunk objects.
        let mut current_buf: Vec<u8> = Vec::new();
        let mut current_hash: Option<Vec<u8>> = None;
        let mut store: Option<forge_core::store::chunk_store::ChunkStore> = None;

        // Channel for handing completed objects to background disk writers.
        // The stream loop validates and reassembles; writers do the slow I/O.
        let (write_tx, write_rx) = crossbeam_channel::bounded::<(ForgeHash, Vec<u8>, bool)>(8);

        // Spawn a pool of blocking writer threads (one per CPU core, capped at 8).
        let num_writers = rayon::current_num_threads().min(8);
        let write_rx = Arc::new(write_rx);
        let write_error: Arc<std::sync::Mutex<Option<String>>> =
            Arc::new(std::sync::Mutex::new(None));
        let mut writer_handles = Vec::with_capacity(num_writers);
        // Store is set on first chunk; writers wait on the channel so this is fine.
        let store_slot: Arc<tokio::sync::OnceCell<forge_core::store::chunk_store::ChunkStore>> =
            Arc::new(tokio::sync::OnceCell::new());

        for _ in 0..num_writers {
            let rx = Arc::clone(&write_rx);
            let err = Arc::clone(&write_error);
            let slot = Arc::clone(&store_slot);
            writer_handles.push(std::thread::spawn(move || {
                while let Ok((hash, data, pre_compressed)) = rx.recv() {
                    // Wait for store to be set (happens on first chunk).
                    let s = loop {
                        if let Some(s) = slot.get() {
                            break s;
                        }
                        std::thread::yield_now();
                    };
                    let result: Result<(), _> = if pre_compressed {
                        s.put_raw_direct(&hash, &data)
                    } else {
                        s.put(&hash, &data).map(|_| ())
                    };
                    if let Err(e) = result {
                        let mut guard = err.lock().unwrap();
                        if guard.is_none() {
                            *guard = Some(e.to_string());
                        }
                        break;
                    }
                }
            }));
        }

        while let Some(chunk) = stream
            .message()
            .await
            .map_err(|e| internal_err("grpc", e))?
        {
            // Read repo from the first chunk.
            if store.is_none() {
                let repo = resolve_repo(&chunk.repo, &caller)?;
                require_repo_write(&caller, &self.user_store, &repo)?;
                self.db.create_repo(&repo, "")
                    .map_err(|e| internal_err("failed to register repo", e))?;
                let s = self.fs.repo_store(&repo);
                // Pre-create all 256 shard dirs so writers skip create_dir_all per object.
                s.ensure_shard_dirs()
                    .map_err(|e| internal_err("shard dirs", e))?;
                let _ = store_slot.set(self.fs.repo_store(&repo));
                store = Some(s);
            }

            if current_hash.as_ref() != Some(&chunk.hash) {
                current_buf.clear();
                current_hash = Some(chunk.hash.clone());
            }

            const MAX_OBJECT_SIZE: usize = 512 * 1024 * 1024;
            if current_buf.len() + chunk.data.len() > MAX_OBJECT_SIZE {
                return Err(Status::resource_exhausted("object exceeds maximum size"));
            }
            current_buf.extend_from_slice(&chunk.data);

            if chunk.is_last {
                let hash_bytes: [u8; 32] = chunk
                    .hash
                    .as_slice()
                    .try_into()
                    .map_err(|_| Status::invalid_argument("invalid hash length"))?;
                let forge_hash = ForgeHash::from_hex(&hex::encode(hash_bytes))
                    .map_err(|e| internal_err("grpc", e))?;

                let pre_compressed = chunk.object_type == 1;
                if pre_compressed {
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
                }

                // Hand off to writer threads — non-blocking unless channel is full,
                // which provides natural backpressure.
                let data = std::mem::take(&mut current_buf);
                write_tx
                    .send((forge_hash, data, pre_compressed))
                    .map_err(|_| Status::internal("writer thread crashed"))?;

                received.push(chunk.hash.clone());
                current_hash = None;

                // Check for write errors periodically.
                if let Some(e) = write_error.lock().unwrap().take() {
                    return Err(internal_err("grpc", e));
                }
            }
        }

        // Drop sender to signal writers to finish, then wait for them.
        drop(write_tx);
        for h in writer_handles {
            let _ = h.join();
        }

        // Final error check.
        if let Some(e) = write_error.lock().unwrap().take() {
            return Err(internal_err("grpc", e));
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

        // Two-stage pipeline (same idea as push):
        //   Stage 1: rayon reads compressed objects from disk in parallel
        //   Stage 2: single thread chunks and sends to gRPC stream
        let (read_tx, read_rx) =
            crossbeam_channel::bounded::<(Vec<u8>, Vec<u8>)>(8);
        // Holds ≤4 MiB ObjectChunks, so 64 slots = 256 MiB max.
        let (tx, rx) = tokio::sync::mpsc::channel(64);

        // Stage 1: parallel disk reads on OS threads.
        let hashes = req.want_hashes;
        std::thread::spawn(move || {
            use rayon::prelude::*;
            hashes.par_iter().for_each(|hash_bytes| {
                let hash_hex = hex::encode(hash_bytes);
                if let Ok(fh) = ForgeHash::from_hex(&hash_hex) {
                    if let Ok(data) = store.get_raw(&fh) {
                        let _ = read_tx.send((hash_bytes.clone(), data));
                    }
                }
            });
        });

        // Stage 2: single thread chunks and sends (preserves per-object ordering).
        std::thread::spawn(move || {
            const CHUNK_SIZE: usize = 4 * 1024 * 1024;
            while let Ok((hash_bytes, compressed)) = read_rx.recv() {
                let total = compressed.len() as u64;
                if compressed.len() <= CHUNK_SIZE {
                    let msg = ObjectChunk {
                        hash: hash_bytes,
                        object_type: 1, // pre-compressed
                        total_size: total,
                        offset: 0,
                        data: compressed,
                        is_last: true,
                        repo: String::new(),
                    };
                    if tx.blocking_send(Ok(msg)).is_err() {
                        return;
                    }
                } else {
                    for (i, slice) in compressed.chunks(CHUNK_SIZE).enumerate() {
                        let off = (i * CHUNK_SIZE) as u64;
                        let is_last = off + slice.len() as u64 == total;
                        let msg = ObjectChunk {
                            hash: hash_bytes.clone(),
                            object_type: 1,
                            total_size: total,
                            offset: off,
                            data: slice.to_vec(),
                            is_last,
                            repo: String::new(),
                        };
                        if tx.blocking_send(Ok(msg)).is_err() {
                            return;
                        }
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

        // Parallelize filesystem stat calls — checking 100K+ paths
        // sequentially is the dominant cost on large pushes.
        let hashes = req.hashes;
        let has = tokio::task::spawn_blocking(move || {
            use rayon::prelude::*;
            hashes
                .par_iter()
                .map(|hash_bytes| {
                    let hash_hex = hex::encode(hash_bytes);
                    match ForgeHash::from_hex(&hash_hex) {
                        Ok(h) => store.has(&h),
                        Err(_) => false,
                    }
                })
                .collect::<Vec<bool>>()
        })
        .await
        .map_err(|e| internal_err("has_objects", e))?;

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
            .map_err(|e| internal_err("grpc", e))?;

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
            .map_err(|e| internal_err("failed to register repo", e))?;

        // Fast-forward guard: for non-force updates against an existing ref,
        // require `new_hash` to be a descendant of `old_hash`. Otherwise a
        // stale client could rewind the branch with a behind-tip.
        let old_is_zero = req.old_hash.iter().all(|&b| b == 0);
        if !req.force && !old_is_zero {
            let old = ForgeHash::from_hex(&hex::encode(&req.old_hash))
                .map_err(|e| internal_err("grpc", e))?;
            let new = ForgeHash::from_hex(&hex::encode(&req.new_hash))
                .map_err(|e| internal_err("grpc", e))?;
            if !is_ancestor_or_equal(&self.object_store(repo), &old, &new) {
                return Ok(Response::new(UpdateRefResponse {
                    success: false,
                    error: "non-fast-forward: new tip is not a descendant of remote tip".into(),
                }));
            }
        }

        let success = self
            .db
            .update_ref(repo, &req.ref_name, &req.old_hash, &req.new_hash, req.force)
            .map_err(|e| internal_err("grpc", e))?;

        // Check push triggers on successful ref update.
        if success {
            audit!(
                action = "ref.update",
                outcome = "success",
                actor_id = caller.user_id(),
                repo = repo,
                ref_name = %req.ref_name,
                old_hash = %hex::encode(&req.old_hash),
                new_hash = %hex::encode(&req.new_hash),
                force = req.force
            );
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
            .map_err(|e| internal_err("grpc", e))?;

        match result {
            Ok(()) => {
                audit!(
                    action = "lock.acquire",
                    outcome = "granted",
                    actor_id = caller.user_id(),
                    repo = repo,
                    path = %req.path,
                    owner = %req.owner
                );
                Ok(Response::new(LockResponse {
                    granted: true,
                    existing_lock: None,
                }))
            }
            Err(lock) => {
                audit!(
                    action = "lock.acquire",
                    outcome = "denied",
                    actor_id = caller.user_id(),
                    repo = repo,
                    path = %req.path,
                    owner = %req.owner,
                    reason = "already held",
                    held_by = %lock.owner
                );
                Ok(Response::new(LockResponse {
                    granted: false,
                    existing_lock: Some(LockInfo {
                        path: lock.path,
                        owner: lock.owner,
                        workspace_id: lock.workspace_id,
                        created_at: lock.created_at,
                        reason: lock.reason,
                    }),
                }))
            }
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

        let success = self
            .db
            .release_lock(repo, &req.path, &req.owner, req.force)
            .map_err(|e| internal_err("grpc", e))?;

        audit!(
            action = if req.force { "lock.force_release" } else { "lock.release" },
            outcome = if success { "success" } else { "noop" },
            actor_id = caller.user_id(),
            repo = repo,
            path = %req.path,
            owner = %req.owner
        );

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
            .map_err(|e| internal_err("grpc", e))?;

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
            .map_err(|e| internal_err("grpc", e))?;

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
            .map_err(|e| internal_err("grpc", e))?;

        let mut repo_infos = Vec::new();
        for r in repos {
            // Get branch info for this repo.
            let all_refs = self
                .db
                .get_all_refs(&r.name)
                .map_err(|e| internal_err("grpc", e))?;

            let branches: Vec<_> = all_refs
                .iter()
                .filter(|(name, _)| name.starts_with("refs/heads/"))
                .collect();
            let branch_count = branches.len() as i32;

            // Try to get last commit info from the default branch.
            let default_branch = if r.default_branch.is_empty() {
                "main".to_string()
            } else {
                r.default_branch
            };
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
            .map_err(|e| internal_err("grpc", e))?;

        if !created {
            return Ok(Response::new(CreateRepoResponse {
                success: false,
                error: format!("repo '{repo}' already exists"),
            }));
        }

        // Ensure the repo's objects directory exists.
        let _store = self.fs.repo_store(&repo);

        audit!(
            action = "repo.create",
            outcome = "success",
            actor_id = caller.user_id(),
            repo = %repo
        );

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
                    error: { tracing::error!(error = %e, "db error"); "internal error".to_string() },
                }));
            }
        }

        // Apply visibility change if provided. Use the post-rename effective
        // name so it works alongside a rename in the same call.
        if !req.visibility.is_empty() {
            let effective = if new_name.is_empty() { repo.clone() } else { new_name.clone() };
            if let Err(e) = self.db.set_repo_visibility(&effective, &req.visibility) {
                tracing::error!(error = %e, "set_repo_visibility failed");
                return Ok(Response::new(UpdateRepoResponse {
                    success: false,
                    error: "visibility update failed".into(),
                }));
            }
        }

        // Apply default_branch change if provided.
        if !req.default_branch.is_empty() {
            let effective = if new_name.is_empty() { repo.clone() } else { new_name.clone() };
            if let Err(e) = self.db.set_default_branch(&effective, &req.default_branch) {
                tracing::error!(error = %e, "set_default_branch failed");
                return Ok(Response::new(UpdateRepoResponse {
                    success: false,
                    error: "default branch update failed".into(),
                }));
            }
        }

        // If renamed, also rename the filesystem directory.
        if !new_name.is_empty() && new_name != repo {
            if let Err(e) = self.fs.rename_repo(&repo, &new_name) {
                tracing::error!(error = %e, "fs.rename_repo failed after db update");
                return Ok(Response::new(UpdateRepoResponse {
                    success: false,
                    error: "internal error during rename".into(),
                }));
            }
        }

        audit!(
            action = "repo.update",
            outcome = "success",
            actor_id = caller.user_id(),
            repo = %repo,
            new_name = %new_name,
            visibility = %req.visibility,
            default_branch = %req.default_branch
        );

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
            .map_err(|e| internal_err("grpc", e))?;

        if !deleted {
            return Ok(Response::new(DeleteRepoResponse {
                success: false,
                error: format!("repo '{repo}' not found"),
            }));
        }

        // Delete from the filesystem.
        if let Err(e) = self.fs.delete_repo(&repo) {
            tracing::error!(error = %e, "fs.delete_repo failed after db delete");
            return Ok(Response::new(DeleteRepoResponse {
                success: false,
                error: "internal error during delete".into(),
            }));
        }

        audit!(
            action = "repo.delete",
            outcome = "success",
            actor_id = caller.user_id(),
            repo = %repo
        );

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
            .map_err(|e| internal_err("grpc", e))?;

        let tip = match tip_bytes {
            Some(b) => ForgeHash::from_hex(&hex::encode(&b))
                .map_err(|e| internal_err("grpc", e))?,
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
            .map_err(|e| internal_err("grpc", e))?;
        let snap = os.get_snapshot(&commit_hash)
            .map_err(|e| internal_err("grpc", e))?;

        // Navigate to the requested path within the tree.
        let mut tree_hash = snap.tree;

        if !req.path.is_empty() {
            for component in req.path.split('/').filter(|c| !c.is_empty()) {
                let tree = os.get_tree(&tree_hash)
                    .map_err(|e| internal_err("grpc", e))?;
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
            .map_err(|e| internal_err("grpc", e))?;

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
            .map_err(|e| internal_err("grpc", e))?;
        let snap = os.get_snapshot(&commit_hash)
            .map_err(|e| internal_err("grpc", e))?;

        // Navigate to the file.
        let mut tree_hash = snap.tree;
        let parts: Vec<&str> = req.path.split('/').filter(|c| !c.is_empty()).collect();
        let (dir_parts, file_name) = parts.split_at(parts.len().saturating_sub(1));

        for component in dir_parts {
            let tree = os.get_tree(&tree_hash)
                .map_err(|e| internal_err("grpc", e))?;
            let entry = tree.entries.iter()
                .find(|e| e.name == *component)
                .ok_or_else(|| Status::not_found(format!("Path not found: {}", req.path)))?;
            tree_hash = entry.hash;
        }

        let tree = os.get_tree(&tree_hash)
            .map_err(|e| internal_err("grpc", e))?;
        let file_entry = tree.entries.iter()
            .find(|e| Some(e.name.as_str()) == file_name.first().copied())
            .ok_or_else(|| Status::not_found(format!("File not found: {}", req.path)))?;

        // Get the file content.
        let content = os.get_blob_data(&file_entry.hash)
            .map_err(|e| internal_err("grpc", e))?;

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
            .map_err(|e| internal_err("grpc", e))?;
        let snap = os.get_snapshot(&commit_hash)
            .map_err(|e| internal_err("grpc", e))?;

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
            .map_err(|e| internal_err("grpc", e))?;
        let repo_names: Vec<String> = repos.iter().map(|r| r.name.clone()).collect();

        // Count total active locks and total branches across all repos.
        let mut total_locks = 0i32;
        let mut total_objects = 0i64;
        let mut total_size_bytes = 0i64;
        for r in &repos {
            let locks = self.db.list_locks(&r.name, "", "")
                .map_err(|e| internal_err("grpc", e))?;
            total_locks += locks.len() as i32;

            // Walk the objects directory for this repo to count objects and size.
            let os = self.object_store(&r.name);
            let objects_dir = os.objects_dir();
            if objects_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&objects_dir) {
                    for prefix_entry in entries.flatten() {
                        if prefix_entry.path().is_dir() {
                            if let Ok(inner) = std::fs::read_dir(prefix_entry.path()) {
                                for obj in inner.flatten() {
                                    total_objects += 1;
                                    total_size_bytes += obj.metadata().map(|m| m.len() as i64).unwrap_or(0);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(Response::new(GetServerInfoResponse {
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs: uptime,
            total_objects,
            total_size_bytes,
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
            .map_err(|e| internal_err("grpc", e))?;
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
            Err(e) => Ok(Response::new(CreateWorkflowResponse { success: false, error: { tracing::error!(error = %e, "db error"); "internal error".to_string() }, id: 0 })),
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
            .map_err(|e| internal_err("grpc", e))?
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
            Err(e) => { tracing::error!(error = %e, "update_workflow"); Ok(Response::new(UpdateWorkflowResponse { success: false, error: "internal error".into() })) },
        }
    }

    async fn delete_workflow(
        &self,
        request: Request<DeleteWorkflowRequest>,
    ) -> Result<Response<DeleteWorkflowResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let workflow = self.db.get_workflow(req.id)
            .map_err(|e| internal_err("grpc", e))?
            .ok_or_else(|| Status::not_found("Workflow not found"))?;
        require_repo_admin(&caller, &self.user_store, &workflow.repo)?;
        match self.db.delete_workflow(req.id) {
            Ok(true) => Ok(Response::new(DeleteWorkflowResponse { success: true, error: String::new() })),
            Ok(false) => Ok(Response::new(DeleteWorkflowResponse { success: false, error: "Workflow not found".into() })),
            Err(e) => { tracing::error!(error = %e, "delete_workflow"); Ok(Response::new(DeleteWorkflowResponse { success: false, error: "internal error".into() })) },
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
            .map_err(|e| internal_err("grpc", e))?
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
            .map_err(|e| internal_err("grpc", e))?
            .map(|h| hex::encode(&h))
            .unwrap_or_default();

        let run_id = self.db.create_run(
            &workflow.repo, workflow.id, "manual", &ref_name, &commit_hash, &req.triggered_by,
        ).map_err(|e| internal_err("grpc", e))?;

        // Queue the run for execution (engine integration in Phase 3).
        if let Some(engine) = &self.workflow_engine {
            let _ = engine.send(run_id);
        }

        audit!(
            action = "workflow.trigger",
            outcome = "success",
            actor_id = caller.user_id(),
            repo = %workflow.repo,
            workflow_id = workflow.id,
            workflow_name = %workflow.name,
            run_id = run_id,
            ref_name = %ref_name
        );

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
            .map_err(|e| internal_err("grpc", e))?;
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
            .map_err(|e| internal_err("grpc", e))?
            .ok_or_else(|| Status::not_found("Run not found"))?;
        require_repo_read(&caller, &self.user_store, &run.repo, self.db.is_repo_public(&run.repo))?;
        let steps = self.db.list_steps(req.run_id)
            .map_err(|e| internal_err("grpc", e))?;
        let artifacts_list = self.db.list_artifacts(req.run_id)
            .map_err(|e| internal_err("grpc", e))?;

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
            .map_err(|e| internal_err("grpc", e))?
            .ok_or_else(|| Status::not_found("Run not found"))?;
        require_repo_write(&caller, &self.user_store, &run.repo)?;
        if run.status != "queued" && run.status != "running" {
            return Ok(Response::new(CancelWorkflowRunResponse {
                success: false, error: format!("Cannot cancel run in '{}' state", run.status),
            }));
        }
        self.db.update_run_status(req.run_id, "cancelled")
            .map_err(|e| internal_err("grpc", e))?;
        audit!(
            action = "workflow.run.cancel",
            outcome = "success",
            actor_id = caller.user_id(),
            repo = %run.repo,
            run_id = req.run_id
        );
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
            .map_err(|e| internal_err("grpc", e))?
            .ok_or_else(|| Status::not_found("Run not found"))?;
        require_repo_read(&caller, &self.user_store, &run.repo, self.db.is_repo_public(&run.repo))?;
        let artifacts = self.db.list_artifacts(req.run_id)
            .map_err(|e| internal_err("grpc", e))?;
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
            .map_err(|e| internal_err("grpc", e))?;
        let mut infos = Vec::new();
        for r in releases {
            let artifact_ids = self.db.get_release_artifact_ids(r.id)
                .map_err(|e| internal_err("grpc", e))?;
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
            .map_err(|e| internal_err("grpc", e))?
            .ok_or_else(|| Status::not_found("Release not found"))?;
        require_repo_read(&caller, &self.user_store, &r.repo, self.db.is_repo_public(&r.repo))?;
        let artifact_ids = self.db.get_release_artifact_ids(r.id)
            .map_err(|e| internal_err("grpc", e))?;
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
            .map_err(|e| internal_err("grpc", e))?;

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
            .map_err(|e| internal_err("grpc", e))?;
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
            .map_err(|e| internal_err("grpc", e))?
            .ok_or_else(|| Status::not_found("Issue not found"))?;
        require_repo_write(&caller, &self.user_store, &issue.repo)?;
        let labels = req.labels.join(",");
        let ok = self.db.update_issue(req.id, &req.title, &req.body, &req.status, &labels, &req.assignee)
            .map_err(|e| internal_err("grpc", e))?;
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
            .map_err(|e| internal_err("grpc", e))?;

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
        ).map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(CreatePullRequestResponse { success: true, error: String::new(), id }))
    }

    async fn update_pull_request(
        &self,
        request: Request<UpdatePullRequestRequest>,
    ) -> Result<Response<UpdatePullRequestResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let pr = self.db.get_pull_request(req.id)
            .map_err(|e| internal_err("grpc", e))?
            .ok_or_else(|| Status::not_found("Pull request not found"))?;
        require_repo_write(&caller, &self.user_store, &pr.repo)?;
        let labels = req.labels.join(",");
        let ok = self.db.update_pull_request(req.id, &req.title, &req.body, &req.status, &labels, &req.assignee)
            .map_err(|e| internal_err("grpc", e))?;
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
            .map_err(|e| internal_err("grpc", e))?
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
            .map_err(|e| internal_err("grpc", e))?
            .ok_or_else(|| Status::not_found(format!("Source branch '{}' not found", pr.source_branch)))?;

        // Get the target branch HEAD hash
        let target_ref = format!("refs/heads/{}", pr.target_branch);
        let target_hash = self.db.get_ref(&pr.repo, &target_ref)
            .map_err(|e| internal_err("grpc", e))?
            .ok_or_else(|| Status::not_found(format!("Target branch '{}' not found", pr.target_branch)))?;

        // Fast-forward merge: CAS-update target branch to point to source
        // branch's HEAD. force = false: a merge that races with a direct
        // push to the target branch should fail and the user can retry.
        let updated = self.db.update_ref(&pr.repo, &target_ref, &target_hash, &source_hash, false)
            .map_err(|e| internal_err("grpc", e))?;

        if !updated {
            return Ok(Response::new(MergePullRequestResponse {
                success: false,
                error: "Failed to update target branch ref (concurrent modification?)".into(),
            }));
        }

        // Mark PR as merged
        self.db.update_pull_request(req.id, "", "", "merged", "", "")
            .map_err(|e| internal_err("grpc", e))?;

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
            .map_err(|e| internal_err("grpc", e))?
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
            .map_err(|e| internal_err("grpc", e))?
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

    // ── Comments ────────────────────────────────────────────────────────────

    async fn list_comments(
        &self,
        request: Request<ListCommentsRequest>,
    ) -> Result<Response<ListCommentsResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo = resolve_repo(&req.repo, &caller)?;
        require_repo_read(&caller, &self.user_store, &repo, self.db.is_repo_public(&repo))?;
        let comments = self.db.list_comments(&repo, req.issue_id, &req.kind)
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(ListCommentsResponse {
            comments: comments.into_iter().map(|c| CommentInfo {
                id: c.id, repo: c.repo, issue_id: c.issue_id, kind: c.kind,
                author: c.author, body: c.body, created_at: c.created_at,
                updated_at: c.updated_at,
            }).collect(),
        }))
    }

    async fn create_comment(
        &self,
        request: Request<CreateCommentRequest>,
    ) -> Result<Response<CreateCommentResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo = resolve_repo(&req.repo, &caller)?;
        require_repo_write(&caller, &self.user_store, &repo)?;
        let id = self.db.create_comment(&repo, req.issue_id, &req.kind, &req.author, &req.body)
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(CreateCommentResponse {
            success: true, error: String::new(), id,
        }))
    }

    async fn update_comment(
        &self,
        request: Request<UpdateCommentRequest>,
    ) -> Result<Response<UpdateCommentResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        // Get the comment to find the repo for authz
        let comment = self.db.get_comment(req.id)
            .map_err(|e| internal_err("grpc", e))?
            .ok_or_else(|| Status::not_found("comment not found"))?;
        require_repo_write(&caller, &self.user_store, &comment.repo)?;
        let ok = self.db.update_comment(req.id, &req.body)
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(UpdateCommentResponse {
            success: ok, error: String::new(),
        }))
    }

    async fn delete_comment(
        &self,
        request: Request<DeleteCommentRequest>,
    ) -> Result<Response<DeleteCommentResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let comment = self.db.get_comment(req.id)
            .map_err(|e| internal_err("grpc", e))?
            .ok_or_else(|| Status::not_found("comment not found"))?;
        require_repo_write(&caller, &self.user_store, &comment.repo)?;
        let ok = self.db.delete_comment(req.id)
            .map_err(|e| internal_err("grpc", e))?;
        Ok(Response::new(DeleteCommentResponse {
            success: ok, error: String::new(),
        }))
    }

    // ── Artifacts (streaming transfer + signed URLs) ──
    //
    // Upload: client-streaming; first chunk carries (run_id, name), rest is
    // data. Gated on repo:write via the run → repo lookup. Download: server-
    // streaming; gated on repo:read. Signed URLs are issued for web-UI
    // downloads so a browser can fetch without re-proving gRPC auth.

    async fn upload_artifact(
        &self,
        request: Request<Streaming<UploadArtifactChunk>>,
    ) -> Result<Response<UploadArtifactResponse>, Status> {
        let caller = caller_of(&request);
        let mut inbound = request.into_inner();

        // Peel the first chunk for metadata.
        let first = inbound
            .message()
            .await
            .map_err(|e| internal_err("upload_artifact_first", e))?
            .ok_or_else(|| Status::invalid_argument("empty upload stream"))?;
        if first.run_id == 0 || first.name.is_empty() {
            return Err(Status::invalid_argument(
                "first upload chunk must set run_id and name",
            ));
        }

        // Auth: look up the run's repo, gate on write.
        let run = self
            .db
            .get_run(first.run_id)
            .map_err(|e| internal_err("upload_artifact_lookup", e))?
            .ok_or_else(|| Status::not_found("run not found"))?;
        require_repo_write(&caller, &self.user_store, &run.repo)?;

        let run_id = first.run_id;
        let name = first.name.clone();

        // Build an AsyncReader that drains the remaining stream. The first
        // chunk's `data` may be non-empty when the whole artifact fits in
        // one frame, so we prepend it.
        use futures::StreamExt;
        let prefix_bytes = first.data;
        let rest = async_stream::stream! {
            if !prefix_bytes.is_empty() {
                yield Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from(prefix_bytes));
            }
            while let Some(chunk) = inbound.next().await {
                match chunk {
                    Ok(c) => {
                        if !c.data.is_empty() {
                            yield Ok(bytes::Bytes::from(c.data));
                        }
                    }
                    Err(e) => {
                        yield Err(std::io::Error::new(std::io::ErrorKind::Other, e.message().to_string()));
                    }
                }
            }
        };
        let reader = tokio_util::io::StreamReader::new(Box::pin(rest));
        let reader: crate::services::artifacts::AsyncReader = Box::pin(reader);

        let handle = self
            .artifacts
            .put(run_id, &name, reader)
            .await
            .map_err(|e| internal_err("upload_artifact_put", e))?;

        let id = self
            .db
            .create_artifact(run_id, &name, &handle.path, handle.size_bytes)
            .map_err(|e| internal_err("upload_artifact_db", e))?;

        Ok(Response::new(UploadArtifactResponse {
            artifact_id: id,
            size_bytes: handle.size_bytes,
        }))
    }

    async fn download_artifact(
        &self,
        request: Request<DownloadArtifactRequest>,
    ) -> Result<Response<Self::DownloadArtifactStream>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();

        let (run_id, _name, path) = self
            .db
            .get_artifact_path(req.artifact_id)
            .map_err(|e| internal_err("download_artifact_lookup", e))?
            .ok_or_else(|| Status::not_found("artifact not found"))?;
        let run = self
            .db
            .get_run(run_id)
            .map_err(|e| internal_err("download_artifact_run", e))?
            .ok_or_else(|| Status::not_found("run not found"))?;
        require_repo_read(&caller, &self.user_store, &run.repo, false)?;

        let mut reader = self
            .artifacts
            .get(&path)
            .await
            .map_err(|e| internal_err("download_artifact_open", e))?;

        let out = async_stream::try_stream! {
            let mut buf = vec![0u8; 4 * 1024 * 1024];
            loop {
                let n = tokio::io::AsyncReadExt::read(&mut reader, &mut buf).await
                    .map_err(|e| Status::internal(format!("read: {e}")))?;
                if n == 0 { break; }
                yield ArtifactChunk { data: buf[..n].to_vec() };
            }
        };
        Ok(Response::new(Box::pin(out)))
    }

    async fn stream_step_logs(
        &self,
        request: Request<StreamStepLogsRequest>,
    ) -> Result<Response<Self::StreamStepLogsStream>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();

        let run = self
            .db
            .get_run(req.run_id)
            .map_err(|e| internal_err("stream_logs_run", e))?
            .ok_or_else(|| Status::not_found("run not found"))?;
        require_repo_read(&caller, &self.user_store, &run.repo, false)?;

        // DB catch-up: pull whatever's already persisted. Filter to one
        // step when caller asked for a specific one.
        let steps = self
            .db
            .list_steps(req.run_id)
            .map_err(|e| internal_err("stream_logs_steps", e))?;
        let want_step = req.step_id;
        let mut initial: Vec<StepLogChunk> = Vec::new();
        for s in &steps {
            if want_step != 0 && s.id != want_step {
                continue;
            }
            if !s.log.is_empty() {
                initial.push(StepLogChunk {
                    step_id: s.id,
                    data: s.log.as_bytes().to_vec(),
                    is_final: s.status == "success"
                        || s.status == "failure"
                        || s.status == "cancelled",
                });
            }
        }

        // If the run is finished or caller said no-follow, just replay and
        // close. Otherwise subscribe for live tail.
        let run_done = run.status == "success"
            || run.status == "failure"
            || run.status == "cancelled";

        if req.no_follow || run_done {
            let out = async_stream::try_stream! {
                for c in initial { yield c; }
            };
            return Ok(Response::new(Box::pin(out)));
        }

        let mut rx = self.log_hub.subscribe(req.run_id);
        let out = async_stream::try_stream! {
            for c in initial { yield c; }
            loop {
                match rx.recv().await {
                    Ok(chunk) => {
                        if want_step != 0 && chunk.step_id != want_step { continue; }
                        yield StepLogChunk {
                            step_id: chunk.step_id,
                            data: chunk.data,
                            is_final: chunk.is_final,
                        };
                    }
                    // Lagging subscribers get a `Lagged` — keep going; the
                    // DB fallback on reconnect covers the gap.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        };
        Ok(Response::new(Box::pin(out)))
    }

    async fn get_artifact_signed_url(
        &self,
        request: Request<GetArtifactSignedUrlRequest>,
    ) -> Result<Response<GetArtifactSignedUrlResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let (run_id, _name, _path) = self
            .db
            .get_artifact_path(req.artifact_id)
            .map_err(|e| internal_err("signed_url_lookup", e))?
            .ok_or_else(|| Status::not_found("artifact not found"))?;
        let run = self
            .db
            .get_run(run_id)
            .map_err(|e| internal_err("signed_url_run", e))?
            .ok_or_else(|| Status::not_found("run not found"))?;
        require_repo_read(&caller, &self.user_store, &run.repo, false)?;

        let ttl = req.ttl_seconds.clamp(60, 3600);
        let token = crate::services::artifacts::signed_url::sign(
            &self.artifact_signer_key,
            req.artifact_id,
            ttl,
        );
        let expires_at = chrono::Utc::now().timestamp() + ttl;
        Ok(Response::new(GetArtifactSignedUrlResponse { token, expires_at }))
    }

    // ── Secrets ──
    //
    // Write-only surface: create / update / delete / list keys. No Read RPC
    // exists by design — values leave the server only via the run executor
    // and are masked in step logs before persistence.

    async fn create_secret(
        &self,
        request: Request<CreateSecretRequest>,
    ) -> Result<Response<CreateSecretResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo = resolve_repo(&req.repo, &caller)?;
        require_repo_admin(&caller, &self.user_store, &repo)?;
        if req.key.is_empty() {
            return Err(Status::invalid_argument("secret key must not be empty"));
        }
        self.secrets
            .put(&repo, &req.key, &req.value)
            .await
            .map_err(|e| internal_err("create_secret", e))?;
        audit!(
            action = "secret.create",
            outcome = "success",
            actor_id = caller.user_id(),
            repo = %repo,
            key = %req.key
        );
        Ok(Response::new(CreateSecretResponse {
            success: true,
            error: String::new(),
        }))
    }

    async fn update_secret(
        &self,
        request: Request<UpdateSecretRequest>,
    ) -> Result<Response<UpdateSecretResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo = resolve_repo(&req.repo, &caller)?;
        require_repo_admin(&caller, &self.user_store, &repo)?;
        if req.key.is_empty() {
            return Err(Status::invalid_argument("secret key must not be empty"));
        }
        self.secrets
            .put(&repo, &req.key, &req.value)
            .await
            .map_err(|e| internal_err("update_secret", e))?;
        audit!(
            action = "secret.update",
            outcome = "success",
            actor_id = caller.user_id(),
            repo = %repo,
            key = %req.key
        );
        Ok(Response::new(UpdateSecretResponse {
            success: true,
            error: String::new(),
        }))
    }

    async fn delete_secret(
        &self,
        request: Request<DeleteSecretRequest>,
    ) -> Result<Response<DeleteSecretResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo = resolve_repo(&req.repo, &caller)?;
        require_repo_admin(&caller, &self.user_store, &repo)?;
        let removed = self
            .secrets
            .delete(&repo, &req.key)
            .await
            .map_err(|e| internal_err("delete_secret", e))?;
        audit!(
            action = "secret.delete",
            outcome = if removed { "success" } else { "noop" },
            actor_id = caller.user_id(),
            repo = %repo,
            key = %req.key
        );
        Ok(Response::new(DeleteSecretResponse {
            success: removed,
            error: if removed {
                String::new()
            } else {
                "secret not found".into()
            },
        }))
    }

    async fn list_secret_keys(
        &self,
        request: Request<ListSecretKeysRequest>,
    ) -> Result<Response<ListSecretKeysResponse>, Status> {
        let caller = caller_of(&request);
        let req = request.into_inner();
        let repo = resolve_repo(&req.repo, &caller)?;
        require_repo_admin(&caller, &self.user_store, &repo)?;
        let rows = self
            .secrets
            .list_keys(&repo)
            .await
            .map_err(|e| internal_err("list_secret_keys", e))?;
        let secrets = rows
            .into_iter()
            .map(|m| SecretMeta {
                repo: m.repo,
                key: m.key,
                created_at: m.created_at,
                updated_at: m.updated_at,
            })
            .collect();
        Ok(Response::new(ListSecretKeysResponse { secrets }))
    }
}
