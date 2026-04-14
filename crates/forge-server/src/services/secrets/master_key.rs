// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Per-server master key used to wrap secret ciphertexts.
//!
//! The key lives at `<base>/secrets/master.key`. It's generated on first use
//! and left at 0600 on Unix so only the server process (and root) can read
//! it. On Windows we rely on NTFS ACLs inherited from the parent dir — the
//! installer is expected to scope the whole data tree to the service account.

use anyhow::{Context, Result};
use rand::RngCore;
use std::path::{Path, PathBuf};

pub const MASTER_KEY_LEN: usize = 32;

/// Canonical on-disk location for the master key.
pub fn key_path(base: &Path) -> PathBuf {
    base.join("secrets").join("master.key")
}

/// Load the master key from disk; generate it on first call.
pub fn load_or_create(base: &Path) -> Result<[u8; MASTER_KEY_LEN]> {
    let path = key_path(base);
    if path.exists() {
        let bytes = std::fs::read(&path)
            .with_context(|| format!("read master key at {}", path.display()))?;
        if bytes.len() != MASTER_KEY_LEN {
            anyhow::bail!(
                "master key at {} is {} bytes, expected {}",
                path.display(),
                bytes.len(),
                MASTER_KEY_LEN
            );
        }
        let mut out = [0u8; MASTER_KEY_LEN];
        out.copy_from_slice(&bytes);
        return Ok(out);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create secrets dir {}", parent.display()))?;
    }
    let mut key = [0u8; MASTER_KEY_LEN];
    rand::thread_rng().fill_bytes(&mut key);
    std::fs::write(&path, key)
        .with_context(|| format!("write master key at {}", path.display()))?;

    // Lock permissions on Unix. On Windows we trust the parent-dir ACLs that
    // the installer sets up — doing it from Rust would need a much bigger
    // Win32 dance than this module deserves.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms)?;
    }

    Ok(key)
}
