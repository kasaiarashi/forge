// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! S3-compatible artifact backend.
//!
//! Real implementation lives behind the `s3-objects` cargo feature
//! (same feature that enables the [S3 object backend](crate::storage::s3_objects)).
//! When the feature is off the stub [`S3ArtifactStore`] still compiles
//! so `backend = "s3"` can be caught at startup with a clear message
//! — mirroring the Postgres preview pattern from Phase 2b.2.
//!
//! The storage layout mirrors the FS backend one-for-one:
//! `<prefix>/<run_id>/<name>`. Every artifact is a single object; there
//! is no multipart split at the artifact boundary. Large artifacts do
//! get multipart *upload*, but the finished object is a single S3
//! object so `get` is a plain GetObject.

use anyhow::{bail, Context, Result};
use async_trait::async_trait;

use crate::config::ArtifactsS3;

use super::{ArtifactHandle, ArtifactStore, AsyncReader};

#[cfg(feature = "s3-objects")]
use aws_sdk_s3::primitives::ByteStream;
#[cfg(feature = "s3-objects")]
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart, Delete, ObjectIdentifier};
#[cfg(feature = "s3-objects")]
use aws_sdk_s3::Client;
#[cfg(feature = "s3-objects")]
use tokio::io::AsyncReadExt;

/// Minimum part size AWS enforces for multipart uploads, except the
/// final part. MinIO/Ceph follow the same rule.
#[cfg(feature = "s3-objects")]
const MULTIPART_CHUNK_BYTES: usize = 8 * 1024 * 1024;

/// Artifacts below this size skip multipart entirely and ship as a
/// single PutObject. Chosen well below S3's 5 MiB minimum-part size so
/// we never pay two round-trips for small build outputs.
#[cfg(feature = "s3-objects")]
const SINGLE_PUT_THRESHOLD: usize = 4 * 1024 * 1024;

pub struct S3ArtifactStore {
    #[allow(dead_code)]
    cfg: ArtifactsS3,
    #[cfg(feature = "s3-objects")]
    client: Client,
    #[cfg(feature = "s3-objects")]
    prefix: String,
}

impl S3ArtifactStore {
    /// Validate config + build an SDK client. When the `s3-objects`
    /// feature is off this only validates; the stub impl below
    /// refuses every method call with a "rebuild with feature"
    /// message so operators see the gap explicitly rather than
    /// getting silent empty responses.
    #[cfg(feature = "s3-objects")]
    pub fn new(cfg: ArtifactsS3) -> Result<Self> {
        if cfg.bucket.is_empty() {
            bail!("artifacts.s3.bucket is required when backend = \"s3\"");
        }
        // Construction needs an async context for credential resolution.
        // The artifacts store is built from the server's async startup
        // path, so `Handle::current()` is always present here — we
        // block_in_place to bridge back to sync for the Ok return.
        let prefix = normalise_prefix(&cfg.prefix);
        let client = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(build_client(&cfg))
        })?;
        Ok(Self {
            cfg,
            client,
            prefix,
        })
    }

    #[cfg(not(feature = "s3-objects"))]
    pub fn new(cfg: ArtifactsS3) -> Result<Self> {
        if cfg.bucket.is_empty() {
            bail!("artifacts.s3.bucket is required when backend = \"s3\"");
        }
        Ok(Self { cfg })
    }

    #[cfg(feature = "s3-objects")]
    fn key_for(&self, run_id: i64, name: &str) -> String {
        format!("{}{}/{}", self.prefix, run_id, name)
    }

    #[cfg(feature = "s3-objects")]
    fn run_prefix(&self, run_id: i64) -> String {
        format!("{}{}/", self.prefix, run_id)
    }
}

#[cfg(feature = "s3-objects")]
async fn build_client(cfg: &ArtifactsS3) -> Result<Client> {
    use aws_config::{BehaviorVersion, Region};
    use aws_sdk_s3::config::Builder as S3ConfigBuilder;

    let mut loader = aws_config::defaults(BehaviorVersion::latest()).region(Region::new(
        if cfg.region.is_empty() {
            "us-east-1".to_string()
        } else {
            cfg.region.clone()
        },
    ));
    // Credentials come from the AWS default chain (env, profile,
    // ECS/EKS/IMDS). `ArtifactsS3` config deliberately doesn't carry
    // static creds — artifact keys are long-lived operator secrets
    // that belong in env / IAM-role, not in forge-server.toml.
    let shared = loader.load().await;
    let mut s3_cfg = S3ConfigBuilder::from(&shared);
    if !cfg.endpoint.is_empty() {
        s3_cfg = s3_cfg.endpoint_url(&cfg.endpoint);
    }
    s3_cfg = s3_cfg.force_path_style(cfg.path_style);
    Ok(Client::from_conf(s3_cfg.build()))
}

#[cfg(feature = "s3-objects")]
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

#[cfg(feature = "s3-objects")]
#[async_trait]
impl ArtifactStore for S3ArtifactStore {
    async fn put(
        &self,
        run_id: i64,
        name: &str,
        mut reader: AsyncReader,
    ) -> Result<ArtifactHandle> {
        let key = self.key_for(run_id, name);

        // Peek-buffer strategy: read up to the single-put threshold
        // into memory. If the reader hits EOF at or under the
        // threshold, ship as a single PutObject (one round-trip,
        // no multipart overhead). Otherwise fall through to
        // streaming multipart with the buffered bytes as part 1.
        let mut buf = Vec::with_capacity(SINGLE_PUT_THRESHOLD);
        let mut chunk = vec![0u8; 64 * 1024];
        while buf.len() < SINGLE_PUT_THRESHOLD {
            let remaining = SINGLE_PUT_THRESHOLD - buf.len();
            let want = chunk.len().min(remaining);
            let n = reader.read(&mut chunk[..want]).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }

        if buf.len() < SINGLE_PUT_THRESHOLD {
            // Hit EOF under the threshold — single put.
            let total = buf.len() as i64;
            let body = ByteStream::from(buf);
            self.client
                .put_object()
                .bucket(&self.cfg.bucket)
                .key(&key)
                .body(body)
                .send()
                .await
                .with_context(|| format!("s3 put_object {}", key))?;
            return Ok(ArtifactHandle {
                path: path_relative(&self.prefix, &key),
                size_bytes: total,
            });
        }

        // Multipart path. `buf` already has the first part (>= 4 MiB);
        // top it up to MULTIPART_CHUNK_BYTES so S3 accepts it (5 MiB
        // minimum for non-final parts).
        let create = self
            .client
            .create_multipart_upload()
            .bucket(&self.cfg.bucket)
            .key(&key)
            .send()
            .await
            .with_context(|| format!("s3 create_multipart_upload {}", key))?;
        let upload_id = create
            .upload_id()
            .ok_or_else(|| anyhow::anyhow!("s3 create_multipart_upload returned no upload id"))?
            .to_string();

        // From here, any failure must attempt an abort to avoid
        // leaving orphaned S3 storage that bills indefinitely.
        let res = run_multipart(
            &self.client,
            &self.cfg.bucket,
            &key,
            &upload_id,
            buf,
            &mut reader,
        )
        .await;

        match res {
            Ok(total) => Ok(ArtifactHandle {
                path: path_relative(&self.prefix, &key),
                size_bytes: total,
            }),
            Err(e) => {
                let _ = self
                    .client
                    .abort_multipart_upload()
                    .bucket(&self.cfg.bucket)
                    .key(&key)
                    .upload_id(&upload_id)
                    .send()
                    .await;
                Err(e)
            }
        }
    }

    async fn get(&self, path: &str) -> Result<AsyncReader> {
        // `path` is the handle we returned from `put` — a key minus
        // our prefix. Re-add it before the GET.
        let key = format!("{}{}", self.prefix, path);
        let resp = self
            .client
            .get_object()
            .bucket(&self.cfg.bucket)
            .key(&key)
            .send()
            .await
            .with_context(|| format!("s3 get_object {}", key))?;
        // Wrap the SDK's AsyncRead impl. The returned reader owns its
        // own connection; the caller drops it when done.
        let reader = resp.body.into_async_read();
        Ok(Box::pin(reader))
    }

    async fn delete_run(&self, run_id: i64) -> Result<()> {
        // List every key under `<prefix>/<run_id>/`, then batch-delete.
        // delete_objects caps at 1000 keys per call; typical CI runs
        // have 1-50 artifacts so one page is usually enough, but we
        // paginate to stay honest.
        let prefix = self.run_prefix(run_id);
        let mut continuation: Option<String> = None;
        loop {
            let mut list = self
                .client
                .list_objects_v2()
                .bucket(&self.cfg.bucket)
                .prefix(&prefix);
            if let Some(ct) = continuation.take() {
                list = list.continuation_token(ct);
            }
            let resp = match list.send().await {
                Ok(r) => r,
                Err(e) => {
                    // Missing prefix → nothing to delete, succeed
                    // silently (called from the sweeper on every tick).
                    tracing::debug!(
                        run_id,
                        error = %e,
                        "s3 delete_run: list_objects_v2 failed"
                    );
                    return Ok(());
                }
            };

            let mut objects: Vec<ObjectIdentifier> = Vec::new();
            for obj in resp.contents() {
                if let Some(k) = obj.key() {
                    if let Ok(oi) = ObjectIdentifier::builder().key(k).build() {
                        objects.push(oi);
                    }
                }
            }
            if !objects.is_empty() {
                let delete = Delete::builder()
                    .set_objects(Some(objects))
                    .build()
                    .context("build Delete")?;
                self.client
                    .delete_objects()
                    .bucket(&self.cfg.bucket)
                    .delete(delete)
                    .send()
                    .await
                    .with_context(|| format!("s3 delete_objects under {}", prefix))?;
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
        Ok(())
    }
}

/// Stream `reader` through the S3 multipart upload initialised in the
/// caller, using `first_part` as part 1. Returns the total byte count
/// on success. Isolated into a free function so the caller can
/// abort_multipart_upload on any error without owning the lifetime
/// juggling inline.
#[cfg(feature = "s3-objects")]
async fn run_multipart(
    client: &Client,
    bucket: &str,
    key: &str,
    upload_id: &str,
    mut first_part: Vec<u8>,
    reader: &mut AsyncReader,
) -> Result<i64> {
    let mut total: i64 = 0;
    let mut completed: Vec<CompletedPart> = Vec::new();
    let mut part_number: i32 = 1;

    // Top up the first part to the minimum multipart chunk size,
    // unless the reader is about to hit EOF (which makes this the
    // final part and AWS allows it to be small).
    while first_part.len() < MULTIPART_CHUNK_BYTES {
        let mut tmp = vec![0u8; 64 * 1024];
        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        first_part.extend_from_slice(&tmp[..n]);
    }

    total += first_part.len() as i64;
    let body = ByteStream::from(first_part);
    let up = client
        .upload_part()
        .bucket(bucket)
        .key(key)
        .upload_id(upload_id)
        .part_number(part_number)
        .body(body)
        .send()
        .await
        .with_context(|| format!("s3 upload_part #{part_number}"))?;
    completed.push(
        CompletedPart::builder()
            .part_number(part_number)
            .set_e_tag(up.e_tag().map(str::to_string))
            .build(),
    );
    part_number += 1;

    // Remaining parts. AWS max is 10,000 parts per upload; at 8 MiB
    // per part that's 80 GiB per artifact, which covers every cooked
    // UE build we've measured.
    loop {
        let mut part_buf: Vec<u8> = Vec::with_capacity(MULTIPART_CHUNK_BYTES);
        let mut scratch = vec![0u8; 64 * 1024];
        while part_buf.len() < MULTIPART_CHUNK_BYTES {
            let want = scratch.len().min(MULTIPART_CHUNK_BYTES - part_buf.len());
            let n = reader.read(&mut scratch[..want]).await?;
            if n == 0 {
                break;
            }
            part_buf.extend_from_slice(&scratch[..n]);
        }
        if part_buf.is_empty() {
            break;
        }
        total += part_buf.len() as i64;
        let body = ByteStream::from(part_buf);
        let up = client
            .upload_part()
            .bucket(bucket)
            .key(key)
            .upload_id(upload_id)
            .part_number(part_number)
            .body(body)
            .send()
            .await
            .with_context(|| format!("s3 upload_part #{part_number}"))?;
        completed.push(
            CompletedPart::builder()
                .part_number(part_number)
                .set_e_tag(up.e_tag().map(str::to_string))
                .build(),
        );
        part_number += 1;
        if part_number > 10_000 {
            anyhow::bail!(
                "s3 multipart: artifact exceeds the 10,000-part cap \
                 (> {} GiB at {} MiB/part)",
                10_000 * MULTIPART_CHUNK_BYTES / (1024 * 1024 * 1024),
                MULTIPART_CHUNK_BYTES / (1024 * 1024),
            );
        }
    }

    let completed_upload = CompletedMultipartUpload::builder()
        .set_parts(Some(completed))
        .build();
    client
        .complete_multipart_upload()
        .bucket(bucket)
        .key(key)
        .upload_id(upload_id)
        .multipart_upload(completed_upload)
        .send()
        .await
        .with_context(|| format!("s3 complete_multipart_upload {}", key))?;
    Ok(total)
}

/// Produce the handle path we hand back to callers — the key with
/// our prefix stripped. The retention sweeper stores this verbatim
/// in the `artifacts.path` DB column, and `get` re-prepends the
/// prefix before the GetObject.
#[cfg(feature = "s3-objects")]
fn path_relative(prefix: &str, key: &str) -> String {
    key.strip_prefix(prefix).unwrap_or(key).to_string()
}

// ── Stub impl for feature-off builds ────────────────────────────────────────
//
// Matches the original stub: every method errors with a clear message.
// main.rs still emits the "rebuild with feature" warning on startup so
// operators learn the gap without waiting for the first upload to fail.

#[cfg(not(feature = "s3-objects"))]
#[async_trait]
impl ArtifactStore for S3ArtifactStore {
    async fn put(&self, _run_id: i64, _name: &str, _reader: AsyncReader) -> Result<ArtifactHandle> {
        bail!(
            "S3 artifact backend requires the `s3-objects` cargo feature. \
             Rebuild forge-server with `--features s3-objects` or set \
             [artifacts] backend = \"fs\"."
        );
    }

    async fn get(&self, _path: &str) -> Result<AsyncReader> {
        bail!("S3 artifact backend requires the `s3-objects` cargo feature.");
    }

    async fn delete_run(&self, _run_id: i64) -> Result<()> {
        // Swallow silently — the retention sweeper and cancel path both
        // call this and both are tolerant of a zero-op delete. Emitting
        // an error every hour from the sweeper would just spam logs.
        Ok(())
    }
}
