// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Trait-compatible KMS secret backend stub.
//!
//! Real KMS-backed storage (AWS KMS, GCP KMS, HashiCorp Vault Transit)
//! would envelope-encrypt each secret's data-key under a master key held
//! by the KMS, keeping ciphertext in SQLite but outsourcing the root of
//! trust. This stub preserves the trait shape and the construction path
//! so a real implementation is a drop-in — it does not store or return
//! secrets.

use anyhow::{bail, Result};
use async_trait::async_trait;

use super::{Secret, SecretBackend, SecretMeta};

/// KMS configuration — minimal shape for the stub. A real backend would
/// carry endpoint, auth mode, key ARN/ID, and an SDK client handle.
#[derive(Debug, Clone, Default)]
pub struct KmsConfig {
    pub provider: String,
    pub key_id: String,
}

pub struct KmsSecretBackend {
    #[allow(dead_code)]
    cfg: KmsConfig,
}

impl KmsSecretBackend {
    pub fn new(cfg: KmsConfig) -> Result<Self> {
        if cfg.key_id.is_empty() {
            bail!("KMS secret backend requires a key_id");
        }
        Ok(Self { cfg })
    }
}

#[async_trait]
impl SecretBackend for KmsSecretBackend {
    async fn put(&self, _repo: &str, _key: &str, _value: &str) -> Result<()> {
        bail!(
            "KMS secret backend is a stub in this build — use the default \
             SQLite+AES-GCM backend or rebuild with KMS support"
        );
    }

    async fn get(&self, _repo: &str, _key: &str) -> Result<Option<Secret>> {
        bail!("KMS secret backend is a stub in this build");
    }

    async fn delete(&self, _repo: &str, _key: &str) -> Result<bool> {
        bail!("KMS secret backend is a stub in this build");
    }

    async fn list_keys(&self, _repo: &str) -> Result<Vec<SecretMeta>> {
        Ok(Vec::new())
    }
}
