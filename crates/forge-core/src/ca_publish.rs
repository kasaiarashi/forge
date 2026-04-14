// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Publish + discover the forge-server cert bundle at a well-known
//! filesystem location so sibling processes (forge-web, tooling, the UE
//! plugin) can reuse it without plumbing config paths by hand.
//!
//! Two use cases share this module:
//!
//! 1. **gRPC client trust root.** forge-web's gRPC channel needs to trust
//!    forge-server's self-signed CA. The old workflow — "install the CA in
//!    the OS root store, hope the user runs as admin" — is fragile on
//!    non-elevated dev boxes. Dropping a copy of `ca.crt` at an
//!    all-users-readable path short-circuits that problem; forge-web finds
//!    it on startup and pins it as the sole TLS trust root.
//!
//! 2. **HTTPS serving cert.** forge-web used to auto-generate its own
//!    CA + leaf at `./forge-web-certs/`, which (a) produced a second trust
//!    root that browsers had to accept and (b) drifted whenever forge-web
//!    was started from a different cwd. If forge-server has already
//!    published a full bundle (`ca.crt` + `server.crt` + `server.key`),
//!    forge-web can serve HTTPS with forge-server's leaf directly — one
//!    CA, one set of SANs, one cert to trust.
//!
//! Target dir resolution (highest priority first):
//!
//!   Windows: `%ProgramData%\Forge\`, then `%LOCALAPPDATA%\Forge\`.
//!   Linux/macOS: `/var/lib/forge/`, then `$HOME/.local/share/forge/`.
//!
//! On publish, we write the bundle to whichever directory accepts the
//! first `create_dir_all` + `write` pair. On discover, we return the
//! first directory whose required files all exist. Failures are logged
//! at `debug` and never propagated — a missing published bundle just
//! means the caller falls back to its own cert generation or OS trust
//! store.
//!
//! **Security note.** On Windows, files written to `%ProgramData%\Forge\`
//! inherit the default ACL, which grants read access to the `Users`
//! group. `server.key` is therefore readable by any local interactive
//! user. Acceptable for the dev / single-host small-team threat model
//! this codebase targets; operators who need stricter isolation should
//! set `server.tls.cert_path` / `key_path` explicitly and keep the key
//! under a restricted directory.

use std::fs;
use std::path::{Path, PathBuf};

/// File names written under the shared directory. Kept in one place so
/// publish and discover agree.
const CA_FILE: &str = "ca.crt";
const LEAF_CERT_FILE: &str = "server.crt";
const LEAF_KEY_FILE: &str = "server.key";

/// A resolved cert trio sitting under a shared directory.
#[derive(Debug, Clone)]
pub struct PublishedBundle {
    pub dir: PathBuf,
    pub ca_cert: PathBuf,
    pub leaf_cert: PathBuf,
    pub leaf_key: PathBuf,
}

/// Candidate *directories*, highest priority first. Empty entries (unset
/// env vars on Windows, unset `HOME` on Unix) are dropped so callers can
/// iterate without guarding every element.
pub fn candidate_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();

    #[cfg(windows)]
    {
        if let Ok(dir) = std::env::var("ProgramData") {
            out.push(PathBuf::from(dir).join("Forge"));
        }
        if let Ok(dir) = std::env::var("LOCALAPPDATA") {
            out.push(PathBuf::from(dir).join("Forge"));
        }
    }

    #[cfg(not(windows))]
    {
        out.push(PathBuf::from("/var/lib/forge"));
        if let Ok(home) = std::env::var("HOME") {
            out.push(PathBuf::from(home).join(".local/share/forge"));
        }
    }

    out
}

/// Legacy helper: list of candidate `ca.crt` paths. Retained because the
/// gRPC-client path only cares about the CA file.
pub fn candidate_paths() -> Vec<PathBuf> {
    candidate_dirs()
        .into_iter()
        .map(|d| d.join(CA_FILE))
        .collect()
}

/// Publish only the CA cert to the first writable shared dir. Used when
/// we don't have (or don't want to expose) the leaf + key.
pub fn publish(src_ca: &Path) -> Option<PathBuf> {
    let bytes = read_source(src_ca)?;
    for dir in candidate_dirs() {
        if !try_mkdir(&dir) {
            continue;
        }
        let target = dir.join(CA_FILE);
        match fs::write(&target, &bytes) {
            Ok(_) => {
                tracing::info!("Published CA to {}", target.display());
                return Some(target);
            }
            Err(e) => {
                tracing::debug!("ca_publish: cannot write {}: {e}", target.display());
                continue;
            }
        }
    }
    None
}

/// Publish the full cert trio (CA + leaf + leaf key) to the first
/// writable shared dir. Returns the resolved bundle on success, `None` if
/// no candidate directory accepted the writes.
///
/// All three files have to land in the *same* directory — we don't mix
/// CA from ProgramData with a key from LOCALAPPDATA, because the discover
/// side looks for the three as a set.
pub fn publish_bundle(
    src_ca: &Path,
    src_leaf_cert: &Path,
    src_leaf_key: &Path,
) -> Option<PublishedBundle> {
    let ca_bytes = read_source(src_ca)?;
    let leaf_bytes = read_source(src_leaf_cert)?;
    let key_bytes = read_source(src_leaf_key)?;

    for dir in candidate_dirs() {
        if !try_mkdir(&dir) {
            continue;
        }
        let ca_dst = dir.join(CA_FILE);
        let leaf_dst = dir.join(LEAF_CERT_FILE);
        let key_dst = dir.join(LEAF_KEY_FILE);

        // Write leaf + key first so discover_bundle never observes a
        // half-published state (ca.crt present, server.crt missing).
        if let Err(e) = fs::write(&leaf_dst, &leaf_bytes) {
            tracing::debug!("ca_publish: cannot write {}: {e}", leaf_dst.display());
            continue;
        }
        if let Err(e) = fs::write(&key_dst, &key_bytes) {
            tracing::debug!("ca_publish: cannot write {}: {e}", key_dst.display());
            continue;
        }
        if let Err(e) = fs::write(&ca_dst, &ca_bytes) {
            tracing::debug!("ca_publish: cannot write {}: {e}", ca_dst.display());
            continue;
        }

        tracing::info!("Published cert bundle to {}", dir.display());
        return Some(PublishedBundle {
            dir,
            ca_cert: ca_dst,
            leaf_cert: leaf_dst,
            leaf_key: key_dst,
        });
    }
    None
}

/// Find the first candidate directory where `ca.crt` exists. Returns
/// only the CA path (legacy shape used by the gRPC client path).
pub fn discover() -> Option<PathBuf> {
    candidate_paths().into_iter().find(|p| p.is_file())
}

/// Find the first candidate directory where the full cert trio exists.
pub fn discover_bundle() -> Option<PublishedBundle> {
    for dir in candidate_dirs() {
        let ca = dir.join(CA_FILE);
        let leaf = dir.join(LEAF_CERT_FILE);
        let key = dir.join(LEAF_KEY_FILE);
        if ca.is_file() && leaf.is_file() && key.is_file() {
            return Some(PublishedBundle {
                dir,
                ca_cert: ca,
                leaf_cert: leaf,
                leaf_key: key,
            });
        }
    }
    None
}

// ── helpers ────────────────────────────────────────────────────────────

fn read_source(path: &Path) -> Option<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            tracing::debug!("ca_publish: cannot read source {}: {e}", path.display());
            None
        }
    }
}

fn try_mkdir(dir: &Path) -> bool {
    match fs::create_dir_all(dir) {
        Ok(_) => true,
        Err(e) => {
            tracing::debug!("ca_publish: cannot create {}: {e}", dir.display());
            false
        }
    }
}
