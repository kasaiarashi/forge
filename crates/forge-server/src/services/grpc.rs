// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use std::sync::Arc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use forge_core::hash::ForgeHash;
use forge_proto::forge::forge_service_server::ForgeService;
use forge_proto::forge::*;

use crate::storage::db::MetadataDb;
use crate::storage::fs::FsStorage;

pub struct ForgeGrpcService {
    pub fs: Arc<FsStorage>,
    pub db: Arc<MetadataDb>,
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

        while let Some(chunk) = stream
            .message()
            .await
            .map_err(|e| Status::internal(e.to_string()))?
        {
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

                self.fs
                    .store
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
        let want_hashes = request.into_inner().want_hashes;
        let fs = Arc::clone(&self.fs);

        let (tx, rx) = tokio::sync::mpsc::channel(32);

        tokio::spawn(async move {
            for hash_bytes in want_hashes {
                let hash_hex = hex::encode(&hash_bytes);
                let forge_hash = match ForgeHash::from_hex(&hash_hex) {
                    Ok(h) => h,
                    Err(_) => continue,
                };

                match fs.store.get(&forge_hash) {
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
        let hashes = request.into_inner().hashes;
        let mut has = Vec::with_capacity(hashes.len());

        for hash_bytes in &hashes {
            let hash_hex = hex::encode(hash_bytes);
            let exists = match ForgeHash::from_hex(&hash_hex) {
                Ok(h) => self.fs.store.has(&h),
                Err(_) => false,
            };
            has.push(exists);
        }

        Ok(Response::new(HasObjectsResponse { has }))
    }

    async fn get_refs(
        &self,
        _request: Request<GetRefsRequest>,
    ) -> Result<Response<GetRefsResponse>, Status> {
        let all_refs = self
            .db
            .get_all_refs()
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

        let success = self
            .db
            .update_ref(&req.ref_name, &req.old_hash, &req.new_hash)
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

        let result = self
            .db
            .acquire_lock(&req.path, &req.owner, &req.workspace_id, &req.reason)
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

        let success = self
            .db
            .release_lock(&req.path, &req.owner, req.force)
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

        let locks = self
            .db
            .list_locks(&req.path_prefix, &req.owner)
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

        // Get all locks for the requested paths.
        let all_locks = self
            .db
            .list_locks("", "")
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
}
