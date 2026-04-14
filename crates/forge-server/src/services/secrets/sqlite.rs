// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! SQLite-backed secret store. Ciphertext is AES-256-GCM with a fresh 12-byte
//! nonce per put; the master key lives outside the DB (see [`master_key`]).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{Context, Result};
use async_trait::async_trait;
use rand::RngCore;
use std::sync::Arc;

use super::{Secret, SecretBackend, SecretMeta};
use crate::storage::db::MetadataDb;

pub struct SqliteSecretBackend {
    db: Arc<MetadataDb>,
    cipher: Aes256Gcm,
}

impl SqliteSecretBackend {
    pub fn new(db: Arc<MetadataDb>, master_key: &[u8; 32]) -> Self {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(master_key));
        Self { db, cipher }
    }

    fn encrypt(&self, plaintext: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
            .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
        Ok((nonce.to_vec(), ct))
    }

    fn decrypt(&self, nonce: &[u8], ct: &[u8]) -> Result<String> {
        let pt = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|e| anyhow::anyhow!("decrypt: {e}"))?;
        String::from_utf8(pt).context("secret value is not UTF-8")
    }
}

#[async_trait]
impl SecretBackend for SqliteSecretBackend {
    async fn put(&self, repo: &str, key: &str, value: &str) -> Result<()> {
        let (nonce, ct) = self.encrypt(value)?;
        let db = Arc::clone(&self.db);
        let repo = repo.to_string();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || db.upsert_secret(&repo, &key, &nonce, &ct))
            .await
            .context("join spawn_blocking")??;
        Ok(())
    }

    async fn get(&self, repo: &str, key: &str) -> Result<Option<Secret>> {
        let db = Arc::clone(&self.db);
        let repo_s = repo.to_string();
        let key_s = key.to_string();
        let row = tokio::task::spawn_blocking(move || db.get_secret(&repo_s, &key_s))
            .await
            .context("join spawn_blocking")??;
        match row {
            None => Ok(None),
            Some((nonce, ct)) => {
                let value = self.decrypt(&nonce, &ct)?;
                Ok(Some(Secret {
                    repo: repo.to_string(),
                    key: key.to_string(),
                    value,
                }))
            }
        }
    }

    async fn delete(&self, repo: &str, key: &str) -> Result<bool> {
        let db = Arc::clone(&self.db);
        let repo = repo.to_string();
        let key = key.to_string();
        let n = tokio::task::spawn_blocking(move || db.delete_secret(&repo, &key))
            .await
            .context("join spawn_blocking")??;
        Ok(n)
    }

    async fn list_keys(&self, repo: &str) -> Result<Vec<SecretMeta>> {
        let db = Arc::clone(&self.db);
        let repo = repo.to_string();
        let rows = tokio::task::spawn_blocking(move || db.list_secret_keys(&repo))
            .await
            .context("join spawn_blocking")??;
        Ok(rows)
    }
}
