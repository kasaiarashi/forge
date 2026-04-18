// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! S3-compatible object backend (Phase 3b.1).
//!
//! Implements [`forge_core::store::backend::ObjectBackend`] over an
//! S3-compatible endpoint — AWS S3 itself, MinIO, Ceph RGW, or any
//! other signer-v4 bucket service. The live-store surface is the full
//! Phase 3a trait: has/get/get_raw/put/put_raw/delete/file_size/iter_all.
//!
//! **Staging is not yet implemented.** The existing FS-backed staging
//! path still owns per-session upload directories; Phase 3b.2 extends
//! the trait with `put_staging` / `promote` so the push path can swap
//! the on-disk rename for an S3 multipart + CopyObject. Until then,
//! S3 is a live-store-only backend and pushes land on FS.
//!
//! **Server wiring is deferred.** Mirroring the 2b.2 → 2b.3 deferral,
//! this module ships the trait impl + MinIO smoke tests without
//! touching the gRPC service. The ~20 call sites that construct
//! `ChunkStore` via `fs.repo_store(repo)` become `Arc<dyn ObjectBackend>`
//! in 3b.2; doing that refactor together with the staging surface keeps
//! the change reviewable.
//!
//! **Async/sync bridge.** The AWS SDK is async; `ObjectBackend` is
//! sync by design (see `backend.rs` module docs). Each method calls
//! `block_in_place` + `Handle::block_on` so the bridge is zero-cost
//! under the server's multi-thread tokio runtime, with a fallback to
//! an owned runtime for contexts without a current handle (tests,
//! admin CLI).

#![cfg(feature = "s3-objects")]

use std::future::Future;
use std::sync::Arc;

use anyhow::{Context, Result};
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{Delete, ObjectIdentifier};
use aws_sdk_s3::Client;
use forge_core::compress;
use forge_core::error::ForgeError;
use forge_core::hash::ForgeHash;
use forge_core::store::backend::ObjectBackend;
use tokio::runtime::{Handle, Runtime};

/// Construction-time configuration for [`S3ObjectBackend`].
///
/// Mirrors the subset of `[objects.s3]` config fields the server needs
/// to reach an S3-compatible endpoint. The fields are intentionally
/// minimal — region/endpoint/bucket/prefix cover AWS S3 + MinIO +
/// Ceph RGW without extra shims.
#[derive(Debug, Clone)]
pub struct S3ObjectBackendConfig {
    /// Bucket name. Required.
    pub bucket: String,
    /// Optional key prefix. Lets multiple forge servers share a
    /// bucket; each prepends its own prefix so their shard trees
    /// don't collide. Trailing `/` is appended if missing.
    pub prefix: String,
    /// AWS region. MinIO ignores the value but the SDK still wants
    /// something non-empty ("us-east-1" is a safe default).
    pub region: String,
    /// Custom endpoint URL. Required for MinIO / Ceph RGW; leave
    /// empty for AWS S3 to use the default endpoint resolver.
    pub endpoint_url: String,
    /// Static access key. Empty → fall back to the default AWS
    /// credential chain (env / profile / IMDS).
    pub access_key_id: String,
    /// Static secret key. Empty → fall back to the default chain.
    pub secret_access_key: String,
    /// Force path-style URLs (`http://host/bucket/key`). MinIO
    /// requires this for standard deploys; set to `false` for AWS
    /// virtual-hosted-style.
    pub path_style: bool,
}

/// S3-backed live object store. Thread-safe via internal `Arc`s on
/// the SDK client; clone freely.
pub struct S3ObjectBackend {
    client: Client,
    bucket: String,
    /// Normalised to always end in `/` (or empty).
    prefix: String,
    /// Owned fallback runtime for contexts where `Handle::try_current`
    /// fails — test binaries without `#[tokio::test]`, admin CLI paths
    /// that run from a synchronous `main`. `None` in normal server
    /// operation where the gRPC runtime is always present.
    fallback_rt: Option<Arc<Runtime>>,
}

impl S3ObjectBackend {
    /// Construct a backend against the given endpoint. Must be called
    /// from within a tokio runtime (the SDK performs async endpoint
    /// resolution at construction); the server's async startup path
    /// satisfies this, as does `#[tokio::test(flavor = "multi_thread")]`.
    pub async fn new(cfg: S3ObjectBackendConfig) -> Result<Self> {
        if cfg.bucket.is_empty() {
            anyhow::bail!("s3_objects: bucket is required");
        }

        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(if cfg.region.is_empty() {
                "us-east-1".to_string()
            } else {
                cfg.region.clone()
            }));
        if !cfg.access_key_id.is_empty() || !cfg.secret_access_key.is_empty() {
            // Static creds take precedence. Empty → default chain (env,
            // profile, ECS/EKS/IMDS). Never synthesise a partial cred.
            let creds = Credentials::new(
                &cfg.access_key_id,
                &cfg.secret_access_key,
                None,
                None,
                "forge-config",
            );
            loader = loader.credentials_provider(creds);
        }
        let shared = loader.load().await;

        let mut s3_cfg = S3ConfigBuilder::from(&shared);
        if !cfg.endpoint_url.is_empty() {
            s3_cfg = s3_cfg.endpoint_url(&cfg.endpoint_url);
        }
        s3_cfg = s3_cfg.force_path_style(cfg.path_style);
        let client = Client::from_conf(s3_cfg.build());

        Ok(Self {
            client,
            bucket: cfg.bucket,
            prefix: normalise_prefix(&cfg.prefix),
            fallback_rt: None,
        })
    }

    /// Constructor variant that attaches an owned fallback runtime.
    /// Used when callers know they might invoke the trait outside an
    /// async context (e.g. from a sync admin CLI).
    pub fn with_fallback_runtime(mut self, rt: Arc<Runtime>) -> Self {
        self.fallback_rt = Some(rt);
        self
    }

    /// Build a view that prepends `extra_prefix` to every key this
    /// backend computes, while sharing the underlying SDK `Client`
    /// (which is `Clone` and internally arc-wrapped). Used by
    /// `S3RepoStorage` to hand out a per-repo scoped backend without
    /// paying a full credential + endpoint resolution per repo —
    /// otherwise every call to `repo_object_backend(repo)` would need
    /// an async construction, which doesn't fit the sync
    /// `RepoStorageBackend` trait.
    pub fn scoped(&self, extra_prefix: &str) -> Self {
        let mut combined = self.prefix.clone();
        combined.push_str(extra_prefix);
        // Always end in `/` so the key formatter produces
        // `<base><repo>/objects/<ab>/<rest>` without double-slashing.
        if !combined.is_empty() && !combined.ends_with('/') {
            combined.push('/');
        }
        Self {
            client: self.client.clone(),
            bucket: self.bucket.clone(),
            prefix: combined,
            fallback_rt: self.fallback_rt.clone(),
        }
    }

    fn key_for(&self, hash: &ForgeHash) -> String {
        let hex = hash.to_hex();
        format!("{}{}/{}", self.prefix, &hex[..2], &hex[2..])
    }

    /// Compose the fully-qualified S3 key prefix for a repo's live
    /// objects. Centralised here so the Phase 3b.5 drain workers and
    /// [`Self::scoped`] share a single source of truth for the layout.
    fn repo_objects_prefix(&self, repo: &str) -> String {
        // Same shape `scoped(&format!("{repo}/objects"))` produces, minus
        // the trailing slash — `list_objects_v2` treats `/` as a literal,
        // so we always append it at the call site.
        format!("{}{}/objects/", self.prefix, repo)
    }

    /// Delete every object under the repo's live-object prefix. Used
    /// by the Phase 3b.5 drain task to finish a queued `delete_repo`
    /// op. Paginates `list_objects_v2` at 1000 keys per page (S3 cap)
    /// and batches `delete_objects` at the same 1000-key cap. Returns
    /// the total deleted key count so the drain can report progress.
    pub async fn drain_delete_repo(&self, repo: &str) -> Result<usize> {
        let prefix = self.repo_objects_prefix(repo);
        let mut total = 0usize;
        let mut continuation: Option<String> = None;
        loop {
            let mut req = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&prefix);
            if let Some(ct) = continuation.take() {
                req = req.continuation_token(ct);
            }
            let resp = req
                .send()
                .await
                .with_context(|| format!("list_objects_v2 {}", prefix))?;

            // Collect this page's keys into one DeleteObjects call.
            let keys: Vec<ObjectIdentifier> = resp
                .contents()
                .iter()
                .filter_map(|o| o.key())
                .map(|k| {
                    ObjectIdentifier::builder()
                        .key(k)
                        .build()
                        .expect("ObjectIdentifier build")
                })
                .collect();
            let page_count = keys.len();
            if !keys.is_empty() {
                let delete = Delete::builder()
                    .set_objects(Some(keys))
                    .build()
                    .context("build Delete for drain")?;
                self.client
                    .delete_objects()
                    .bucket(&self.bucket)
                    .delete(delete)
                    .send()
                    .await
                    .with_context(|| format!("delete_objects batch for {}", prefix))?;
            }
            total += page_count;

            if resp.is_truncated().unwrap_or(false) {
                continuation = resp.next_continuation_token().map(str::to_string);
                if continuation.is_none() {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(total)
    }

    /// Copy every object under `old_repo`'s live-object prefix to
    /// `new_repo`'s prefix, then delete the originals. Same pagination
    /// + batched-delete shape as [`Self::drain_delete_repo`].
    ///
    /// Idempotent on resume: if a crash leaves some keys under the
    /// new prefix AND some still under the old, re-running the drain
    /// just re-copies + re-deletes whatever is still under old. S3
    /// CopyObject is safe to retry because keys are content-addressed
    /// — a partial copy becomes a full copy, never a corruption.
    pub async fn drain_rename_repo(
        &self,
        old_repo: &str,
        new_repo: &str,
    ) -> Result<usize> {
        if old_repo == new_repo {
            // No-op: rename to self shouldn't have been enqueued, but
            // make sure the drain completes cleanly if it was.
            return Ok(0);
        }
        let old_prefix = self.repo_objects_prefix(old_repo);
        let new_prefix = self.repo_objects_prefix(new_repo);
        let mut total = 0usize;
        let mut continuation: Option<String> = None;
        loop {
            let mut req = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&old_prefix);
            if let Some(ct) = continuation.take() {
                req = req.continuation_token(ct);
            }
            let resp = req
                .send()
                .await
                .with_context(|| format!("list_objects_v2 {}", old_prefix))?;

            let mut to_delete: Vec<ObjectIdentifier> = Vec::new();
            for obj in resp.contents() {
                let Some(src_key) = obj.key() else { continue };
                let Some(tail) = src_key.strip_prefix(&old_prefix) else {
                    continue;
                };
                let dst_key = format!("{}{}", new_prefix, tail);
                // CopyObject's `CopySource` is `{bucket}/{key}`; SDK
                // percent-encodes when we pass a raw string.
                let copy_source = format!("{}/{}", self.bucket, src_key);
                self.client
                    .copy_object()
                    .bucket(&self.bucket)
                    .key(&dst_key)
                    .copy_source(&copy_source)
                    .send()
                    .await
                    .with_context(|| {
                        format!("copy_object {src_key} -> {dst_key}")
                    })?;
                to_delete.push(
                    ObjectIdentifier::builder()
                        .key(src_key)
                        .build()
                        .expect("ObjectIdentifier build"),
                );
            }
            let page_count = to_delete.len();
            if !to_delete.is_empty() {
                let delete = Delete::builder()
                    .set_objects(Some(to_delete))
                    .build()
                    .context("build Delete for rename drain")?;
                self.client
                    .delete_objects()
                    .bucket(&self.bucket)
                    .delete(delete)
                    .send()
                    .await
                    .with_context(|| {
                        format!("delete_objects after copy for {old_prefix}")
                    })?;
            }
            total += page_count;

            if resp.is_truncated().unwrap_or(false) {
                continuation = resp.next_continuation_token().map(str::to_string);
                if continuation.is_none() {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(total)
    }

    /// Bridge: run `fut` to completion regardless of whether we're
    /// already inside a tokio runtime. Inside one, we `block_in_place`
    /// so the current worker hands off; outside, we use the owned
    /// fallback runtime or spin up a throwaway one.
    fn block_on<F: Future>(&self, fut: F) -> F::Output {
        if let Ok(handle) = Handle::try_current() {
            tokio::task::block_in_place(|| handle.block_on(fut))
        } else if let Some(rt) = self.fallback_rt.as_ref() {
            rt.block_on(fut)
        } else {
            // Last-resort: current-thread throwaway. Creating a
            // runtime per call is expensive (~ms); callers who hit
            // this path in the hot loop should attach a fallback.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("construct tokio runtime for s3 call");
            rt.block_on(fut)
        }
    }
}

fn normalise_prefix(p: &str) -> String {
    if p.is_empty() {
        return String::new();
    }
    if p.ends_with('/') {
        p.to_string()
    } else {
        format!("{p}/")
    }
}

impl ObjectBackend for S3ObjectBackend {
    fn has(&self, hash: &ForgeHash) -> bool {
        self.block_on(async {
            self.client
                .head_object()
                .bucket(&self.bucket)
                .key(self.key_for(hash))
                .send()
                .await
                .is_ok()
        })
    }

    fn get(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        let compressed = self.get_raw(hash)?;
        let data = compress::decompress(&compressed)?;
        // Same integrity check the FS backend does — a partial / tampered
        // blob would otherwise silently round-trip as wrong data.
        let actual = ForgeHash::from_bytes(&data);
        if actual != *hash {
            return Err(ForgeError::Other(format!(
                "integrity error: object {} has hash {} after s3 fetch",
                hash.to_hex(),
                actual.to_hex()
            )));
        }
        Ok(data)
    }

    fn get_raw(&self, hash: &ForgeHash) -> Result<Vec<u8>, ForgeError> {
        let key = self.key_for(hash);
        let res = self.block_on(async {
            self.client
                .get_object()
                .bucket(&self.bucket)
                .key(&key)
                .send()
                .await
        });
        let resp = match res {
            Ok(r) => r,
            Err(err) => {
                // Map NotFound explicitly so callers can distinguish
                // "absent" from "S3 is broken".
                if is_not_found(&err) {
                    return Err(ForgeError::ObjectNotFound(hash.to_hex()));
                }
                return Err(ForgeError::Other(format!("s3 get_object: {err}")));
            }
        };
        let bytes = self
            .block_on(async { resp.body.collect().await })
            .map_err(|e| ForgeError::Other(format!("s3 read body: {e}")))?
            .into_bytes();
        Ok(bytes.to_vec())
    }

    fn put(&self, hash: &ForgeHash, data: &[u8]) -> Result<bool, ForgeError> {
        if self.has(hash) {
            return Ok(false);
        }
        let compressed = compress::compress(data)?;
        self.put_raw(hash, &compressed)
    }

    fn put_raw(&self, hash: &ForgeHash, compressed: &[u8]) -> Result<bool, ForgeError> {
        if self.has(hash) {
            return Ok(false);
        }
        let key = self.key_for(hash);
        let body = ByteStream::from(compressed.to_vec());
        let res = self.block_on(async {
            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(&key)
                .body(body)
                .send()
                .await
        });
        match res {
            Ok(_) => Ok(true),
            Err(e) => Err(ForgeError::Other(format!("s3 put_object: {e}"))),
        }
    }

    fn delete(&self, hash: &ForgeHash) -> Result<bool, ForgeError> {
        // `delete_object` is idempotent on S3 — absent key returns 204,
        // not 404. Check existence first so we can faithfully report
        // the bool the FS backend returns (telling the GC whether it
        // actually reclaimed anything).
        if !self.has(hash) {
            return Ok(false);
        }
        let key = self.key_for(hash);
        let res = self.block_on(async {
            self.client
                .delete_object()
                .bucket(&self.bucket)
                .key(&key)
                .send()
                .await
        });
        match res {
            Ok(_) => Ok(true),
            Err(e) => Err(ForgeError::Other(format!("s3 delete_object: {e}"))),
        }
    }

    fn file_size(&self, hash: &ForgeHash) -> Option<u64> {
        self.block_on(async {
            self.client
                .head_object()
                .bucket(&self.bucket)
                .key(self.key_for(hash))
                .send()
                .await
                .ok()
                .and_then(|h| h.content_length())
                .and_then(|n| u64::try_from(n).ok())
        })
    }

    fn iter_all<'a>(
        &'a self,
    ) -> Result<Box<dyn Iterator<Item = Result<ForgeHash, ForgeError>> + 'a>, ForgeError> {
        // Paginate the whole bucket under our prefix. S3 caps each
        // page at 1000 keys; we collect into a Vec because the trait
        // hands back a boxed iterator — streaming across async page
        // boundaries from a sync iterator is significantly more work
        // than it's worth for GC, which already materialises the
        // marked set anyway.
        let prefix = self.prefix.clone();
        let hashes = self.block_on(async {
            let mut out: Vec<Result<ForgeHash, ForgeError>> = Vec::new();
            let mut continuation: Option<String> = None;
            loop {
                let mut req = self
                    .client
                    .list_objects_v2()
                    .bucket(&self.bucket)
                    .prefix(&prefix);
                if let Some(ct) = continuation.take() {
                    req = req.continuation_token(ct);
                }
                let resp = match req.send().await {
                    Ok(r) => r,
                    Err(e) => {
                        return Err(ForgeError::Other(format!("s3 list_objects: {e}")));
                    }
                };
                for obj in resp.contents() {
                    let Some(key) = obj.key() else {
                        continue;
                    };
                    // Strip our prefix, then expect `<ab>/<rest>`.
                    let Some(rest) = key.strip_prefix(&prefix) else {
                        continue;
                    };
                    let Some((shard, tail)) = rest.split_once('/') else {
                        continue;
                    };
                    if shard.len() != 2 || !shard.chars().all(|c| c.is_ascii_hexdigit()) {
                        continue;
                    }
                    if tail.len() != 62 || !tail.chars().all(|c| c.is_ascii_hexdigit()) {
                        continue;
                    }
                    let hex = format!("{shard}{tail}");
                    match ForgeHash::from_hex(&hex) {
                        Ok(h) => out.push(Ok(h)),
                        Err(e) => out.push(Err(e)),
                    }
                }
                if resp.is_truncated().unwrap_or(false) {
                    continuation = resp.next_continuation_token().map(str::to_string);
                    if continuation.is_none() {
                        break;
                    }
                } else {
                    break;
                }
            }
            Ok::<_, ForgeError>(out)
        })?;

        Ok(Box::new(hashes.into_iter()))
    }
}

fn is_not_found(err: &aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::get_object::GetObjectError>) -> bool {
    matches!(
        err,
        aws_sdk_s3::error::SdkError::ServiceError(se)
            if matches!(
                se.err(),
                aws_sdk_s3::operation::get_object::GetObjectError::NoSuchKey(_)
            )
    )
}

/// Helper for `forge-server gc` and future 3b.2 wiring: delete a
/// batch of hashes in one S3 round-trip (up to 1000 per call, S3's
/// cap). Not part of the trait because the FS backend has no such
/// batching; the generic GC loop calls `delete` per-hash either way.
pub fn delete_batch(
    backend: &S3ObjectBackend,
    hashes: impl IntoIterator<Item = ForgeHash>,
) -> Result<usize> {
    let mut keys: Vec<ObjectIdentifier> = Vec::new();
    for h in hashes {
        let ident = ObjectIdentifier::builder()
            .key(backend.key_for(&h))
            .build()
            .context("build ObjectIdentifier")?;
        keys.push(ident);
    }
    if keys.is_empty() {
        return Ok(0);
    }
    let count = keys.len();
    backend.block_on(async {
        let delete = Delete::builder()
            .set_objects(Some(keys))
            .build()
            .context("build Delete")?;
        backend
            .client
            .delete_objects()
            .bucket(&backend.bucket)
            .delete(delete)
            .send()
            .await
            .context("s3 delete_objects")?;
        Ok::<_, anyhow::Error>(())
    })?;
    Ok(count)
}

// ── Integration smoke test ───────────────────────────────────────────────────
//
// Only compiled under the `s3-objects-tests` feature so the normal dev
// `cargo test` doesn't hammer a container at every run. CI points
// `FORGE_S3_ENDPOINT` and `FORGE_S3_BUCKET` at a MinIO instance.

#[cfg(all(test, feature = "s3-objects-tests"))]
mod tests {
    use super::*;

    fn env_or_skip(var: &str) -> Option<String> {
        std::env::var(var).ok().filter(|v| !v.is_empty())
    }

    fn load_config() -> Option<S3ObjectBackendConfig> {
        let bucket = env_or_skip("FORGE_S3_BUCKET")?;
        Some(S3ObjectBackendConfig {
            bucket,
            prefix: std::env::var("FORGE_S3_PREFIX")
                .unwrap_or_else(|_| "forge-tests/".into()),
            region: std::env::var("FORGE_S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
            endpoint_url: std::env::var("FORGE_S3_ENDPOINT").unwrap_or_default(),
            access_key_id: std::env::var("FORGE_S3_ACCESS_KEY").unwrap_or_default(),
            secret_access_key: std::env::var("FORGE_S3_SECRET_KEY").unwrap_or_default(),
            path_style: std::env::var("FORGE_S3_PATH_STYLE")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(true),
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn s3_roundtrip_smoke() {
        let Some(cfg) = load_config() else {
            eprintln!(
                "FORGE_S3_BUCKET not set — skipping S3 smoke test. \
                 Enable by pointing at a MinIO / S3 endpoint."
            );
            return;
        };
        let backend = S3ObjectBackend::new(cfg).await.unwrap();

        let payload = b"hello s3 forge backend";
        let hash = ForgeHash::from_bytes(payload);

        assert!(!backend.has(&hash), "bucket must not carry this key yet");
        assert!(backend.put(&hash, payload).unwrap(), "fresh insert");
        assert!(backend.has(&hash));
        assert!(!backend.put(&hash, payload).unwrap(), "dedup on repeat");

        let got = backend.get(&hash).unwrap();
        assert_eq!(got.as_slice(), payload);

        let mut found = false;
        for item in backend.iter_all().unwrap() {
            if item.unwrap() == hash {
                found = true;
            }
        }
        assert!(found, "iter_all must surface the freshly put hash");

        assert!(backend.delete(&hash).unwrap(), "deleted");
        assert!(!backend.delete(&hash).unwrap(), "idempotent second delete");
        assert!(!backend.has(&hash));
    }
}
