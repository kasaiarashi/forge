// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Secrets subsystem. Pluggable backend trait with an AES-GCM SQLite default.
//!
//! Secret *values* flow outward only to the run executor; no RPC returns them
//! to clients. Callers can create, update, delete, and list *keys* only. The
//! trait is async so future backends (KMS, Vault) slot in without reshaping
//! the call sites.

use anyhow::Result;
use async_trait::async_trait;

pub mod sqlite;
pub mod master_key;
pub mod mask;

/// One stored secret. `value` is plaintext — only produced inside
/// [`SecretBackend::get`] and immediately consumed by the run executor or the
/// masking layer. Never serialised over the wire.
#[derive(Debug, Clone)]
pub struct Secret {
    pub repo: String,
    pub key: String,
    pub value: String,
}

/// Summary of a secret suitable for listing. No value — the whole point of
/// this type is that it's safe to hand to any authenticated caller.
#[derive(Debug, Clone)]
pub struct SecretMeta {
    pub repo: String,
    pub key: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[async_trait]
pub trait SecretBackend: Send + Sync {
    async fn put(&self, repo: &str, key: &str, value: &str) -> Result<()>;
    async fn get(&self, repo: &str, key: &str) -> Result<Option<Secret>>;
    async fn delete(&self, repo: &str, key: &str) -> Result<bool>;
    async fn list_keys(&self, repo: &str) -> Result<Vec<SecretMeta>>;
}
