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

use crate::storage::db::MetadataDb;
use crate::storage::fs::FsStorage;

pub struct ForgeGrpcService {
    pub fs: Arc<FsStorage>,
    pub db: Arc<MetadataDb>,
    pub start_time: Instant,
}

/// Normalize repo name: empty string defaults to "default".
fn repo_name(repo: &str) -> &str {
    if repo.is_empty() { "default" } else { repo }
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
                let repo = repo_name(&chunk.repo);
                store = Some(self.fs.repo_store(repo));
            }

            if current_hash.as_ref() != Some(&chunk.hash) {
                // New object starting.
                current_buf.clear();
                current_hash = Some(chunk.hash.clone());
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

                store
                    .as_ref()
                    .unwrap()
                    .put(&forge_hash, &current_buf)
                    .map_err(|e| Status::internal(e.to_string()))?;

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
        let req = request.into_inner();
        let repo = repo_name(&req.repo).to_string();
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
        let req = request.into_inner();
        let repo = repo_name(&req.repo);
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
        let req = request.into_inner();
        let repo = repo_name(&req.repo);

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
        let req = request.into_inner();
        let repo = repo_name(&req.repo);

        let success = self
            .db
            .update_ref(repo, &req.ref_name, &req.old_hash, &req.new_hash)
            .map_err(|e| Status::internal(e.to_string()))?;

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
        let req = request.into_inner();
        let repo = repo_name(&req.repo);

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
        let req = request.into_inner();
        let repo = repo_name(&req.repo);

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
        let req = request.into_inner();
        let repo = repo_name(&req.repo);

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
        let req = request.into_inner();
        let repo = repo_name(&req.repo);

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
        _request: Request<ListReposRequest>,
    ) -> Result<Response<ListReposResponse>, Status> {
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
            });
        }

        Ok(Response::new(ListReposResponse { repos: repo_infos }))
    }

    async fn create_repo(
        &self,
        request: Request<CreateRepoRequest>,
    ) -> Result<Response<CreateRepoResponse>, Status> {
        let req = request.into_inner();

        if req.name.is_empty() {
            return Ok(Response::new(CreateRepoResponse {
                success: false,
                error: "repo name cannot be empty".into(),
            }));
        }

        // Create the repo record in the database.
        let created = self
            .db
            .create_repo(&req.name, &req.description)
            .map_err(|e| Status::internal(e.to_string()))?;

        if !created {
            return Ok(Response::new(CreateRepoResponse {
                success: false,
                error: format!("repo '{}' already exists", req.name),
            }));
        }

        // Ensure the repo's objects directory exists.
        let _store = self.fs.repo_store(&req.name);

        Ok(Response::new(CreateRepoResponse {
            success: true,
            error: String::new(),
        }))
    }

    async fn update_repo(
        &self,
        request: Request<UpdateRepoRequest>,
    ) -> Result<Response<UpdateRepoResponse>, Status> {
        let req = request.into_inner();

        if req.name.is_empty() {
            return Ok(Response::new(UpdateRepoResponse {
                success: false,
                error: "repo name cannot be empty".into(),
            }));
        }

        // Update the database record.
        match self.db.update_repo(&req.name, &req.new_name, &req.description) {
            Ok(true) => {}
            Ok(false) => {
                return Ok(Response::new(UpdateRepoResponse {
                    success: false,
                    error: format!("repo '{}' not found", req.name),
                }));
            }
            Err(e) => {
                return Ok(Response::new(UpdateRepoResponse {
                    success: false,
                    error: e.to_string(),
                }));
            }
        }

        // If renamed, also rename the filesystem directory.
        if !req.new_name.is_empty() && req.new_name != req.name {
            if let Err(e) = self.fs.rename_repo(&req.name, &req.new_name) {
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
        let req = request.into_inner();

        if req.name.is_empty() {
            return Ok(Response::new(DeleteRepoResponse {
                success: false,
                error: "repo name cannot be empty".into(),
            }));
        }

        // Delete from the database.
        let deleted = self
            .db
            .delete_repo(&req.name)
            .map_err(|e| Status::internal(e.to_string()))?;

        if !deleted {
            return Ok(Response::new(DeleteRepoResponse {
                success: false,
                error: format!("repo '{}' not found", req.name),
            }));
        }

        // Delete from the filesystem.
        if let Err(e) = self.fs.delete_repo(&req.name) {
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
        let req = request.into_inner();
        let repo = repo_name(&req.repo);
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
                    hash: current.short(),
                    message: snap.message.clone(),
                    author_name: snap.author.name.clone(),
                    author_email: snap.author.email.clone(),
                    timestamp: snap.timestamp.timestamp(),
                    parent_hashes: snap.parents.iter().map(|p| p.short()).collect(),
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
        let req = request.into_inner();
        let repo = repo_name(&req.repo);
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
            TreeEntryInfo {
                name: e.name.clone(),
                kind: match e.kind {
                    forge_core::object::tree::EntryKind::File => "file".into(),
                    forge_core::object::tree::EntryKind::Directory => "directory".into(),
                    forge_core::object::tree::EntryKind::Symlink => "symlink".into(),
                },
                hash: e.hash.short(),
                size: e.size,
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
        let req = request.into_inner();
        let repo = repo_name(&req.repo);
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

        Ok(Response::new(GetFileContentResponse {
            content,
            size,
            is_binary,
            hash: file_entry.hash.short(),
        }))
    }

    async fn get_commit_detail(
        &self,
        request: Request<GetCommitDetailRequest>,
    ) -> Result<Response<GetCommitDetailResponse>, Status> {
        let req = request.into_inner();
        let repo = repo_name(&req.repo);
        let os = self.object_store(repo);

        let commit_hash = ForgeHash::from_hex(&req.commit_hash)
            .map_err(|e| Status::internal(e.to_string()))?;
        let snap = os.get_snapshot(&commit_hash)
            .map_err(|e| Status::internal(e.to_string()))?;

        let commit = CommitInfo {
            hash: commit_hash.short(),
            message: snap.message.clone(),
            author_name: snap.author.name.clone(),
            author_email: snap.author.email.clone(),
            timestamp: snap.timestamp.timestamp(),
            parent_hashes: snap.parents.iter().map(|p| p.short()).collect(),
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
        _request: Request<GetServerInfoRequest>,
    ) -> Result<Response<GetServerInfoResponse>, Status> {
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
}
