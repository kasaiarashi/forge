// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under BSL 1.1.

//! Edge-replica write redirection.
//!
//! Phase 7e ships read-only edge replicas that reject write RPCs with
//! `Status::failed_precondition` carrying the primary's URL — both as a
//! human-readable suffix on the message *and* as a structured metadata
//! header (`x-forge-upstream-write-url`). The structured header is the
//! one we consume: regex-parsing a free-form error message is brittle
//! and would drift silently if the server reformats its error text.
//!
//! ## How the redirect becomes "transparent"
//!
//! There is no in-band, stream-safe auto-retry — streaming writes
//! (`PushObjects`) can't be replayed after the first `Send::poll_ready`
//! consumes the gRPC frame, and forcing every call site to wrap the
//! RPC in a retry closure balloons the surface area. Instead we use a
//! small on-disk **cache** (`~/.forge/edge_redirect.json`):
//!
//! 1. The first write against an edge replica fails with the hint.
//! 2. `forge-cli`'s pretty-error path reads the hint, records
//!    `edge_url → primary_url` in the cache, and prints a clear
//!    "retry against the primary" message to the user.
//! 3. Every subsequent `connect_forge_write(edge_url)` consults the
//!    cache *before* dialing and transparently redirects to the
//!    primary. The user sees the edge URL in their workspace config
//!    but all write traffic silently lands on the primary.
//!
//! One failure per edge URL per client machine — very cheap — and no
//! per-command retry logic to maintain. Cache entries are keyed by the
//! resolved (post-TOFU) server URL so a workspace whose remote points
//! at `https://edge.example` gets redirected regardless of which
//! `forge-cli` command kicks off.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Metadata header key set by the server's edge layer on every
/// write-rejection Status. Kept in lockstep with
/// `forge-server::services::edge::EDGE_UPSTREAM_HEADER`.
pub const EDGE_UPSTREAM_HEADER: &str = "x-forge-upstream-write-url";

/// Inspect a gRPC `Status` for the edge-redirect metadata header. Only
/// returns `Some` when the status is `FailedPrecondition` *and* the
/// header is present, so callers can't mistake an unrelated precondition
/// failure for an edge redirect.
pub fn extract_upstream_hint(status: &tonic::Status) -> Option<String> {
    if status.code() != tonic::Code::FailedPrecondition {
        return None;
    }
    let md = status.metadata().get(EDGE_UPSTREAM_HEADER)?;
    let s = md.to_str().ok()?.trim();
    if s.is_empty() {
        return None;
    }
    Some(s.to_string())
}

/// Persisted edge → primary map. Lives at `~/.forge/edge_redirect.json`.
/// Deliberately a flat JSON object keyed by edge URL — easy for
/// operators to inspect, edit by hand, or wipe with `rm`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    /// Edge URL → primary write URL. Both stored verbatim as the
    /// caller passed them (we do not canonicalise `https://…/` trailing
    /// slashes; the CLI resolver already normalises before calling in).
    redirects: HashMap<String, String>,
}

fn cache_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".forge").join("edge_redirect.json"))
}

fn load() -> CacheFile {
    let Some(path) = cache_path() else {
        return CacheFile::default();
    };
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => CacheFile::default(),
        Err(_) => CacheFile::default(),
    }
}

fn save(cache: &CacheFile) -> io::Result<()> {
    let Some(path) = cache_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(cache)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    // Atomic replace — avoid leaving a truncated file on crash.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Resolve a server URL to the best known write target. If the URL is a
/// known edge replica, returns the cached primary URL; otherwise
/// returns the input verbatim. Never fails — a missing / malformed
/// cache is treated as empty (writes will fail once against the edge
/// and repopulate the cache).
pub fn resolve_write_target(server_url: &str) -> String {
    let cache = load();
    cache
        .redirects
        .get(server_url)
        .cloned()
        .unwrap_or_else(|| server_url.to_string())
}

/// Record that `edge_url` is a read-only edge backed by `upstream_url`.
/// Called from the CLI's pretty-error path on the first write failure
/// so subsequent runs skip the round-trip. Best-effort: a failed write
/// to the cache file logs but does not propagate — the user can still
/// complete the operation by re-running with `--server upstream_url`
/// manually.
pub fn record_edge_upstream(edge_url: &str, upstream_url: &str) {
    if edge_url == upstream_url {
        return;
    }
    let mut cache = load();
    cache
        .redirects
        .insert(edge_url.to_string(), upstream_url.to_string());
    if let Err(e) = save(&cache) {
        tracing::debug!(error = %e, "failed to persist edge redirect cache");
    }
}

/// Remove any cached redirect for `edge_url`. Exposed so operators can
/// invalidate the cache after a failover (primary itself moves).
pub fn forget_edge_upstream(edge_url: &str) {
    let mut cache = load();
    if cache.redirects.remove(edge_url).is_some() {
        let _ = save(&cache);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Status;

    #[test]
    fn extract_hint_from_failed_precondition_status() {
        let mut s = Status::failed_precondition("edge refused");
        s.metadata_mut().insert(
            EDGE_UPSTREAM_HEADER,
            "https://primary.example".parse().unwrap(),
        );
        assert_eq!(
            extract_upstream_hint(&s).as_deref(),
            Some("https://primary.example")
        );
    }

    #[test]
    fn extract_hint_rejects_non_precondition_codes() {
        let mut s = Status::unavailable("oops");
        s.metadata_mut().insert(
            EDGE_UPSTREAM_HEADER,
            "https://primary.example".parse().unwrap(),
        );
        // Even if a (misbehaving) server attached the header, don't
        // treat a transient Unavailable as an edge redirect.
        assert_eq!(extract_upstream_hint(&s), None);
    }

    #[test]
    fn extract_hint_absent_without_header() {
        let s = Status::failed_precondition("something else");
        assert_eq!(extract_upstream_hint(&s), None);
    }
}
