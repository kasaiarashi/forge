// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Install the auto-generated CA into the Windows trust store so TLS clients
//! stop failing with `invalid peer certificate: BadSignature`.
//!
//! The primary motivator is forge-web: its gRPC client calls
//! `ClientTlsConfig::with_native_roots()` when no explicit CA path is
//! configured, which on Windows means "trust whatever is in the OS root
//! store". Our auto-generated CA is self-signed and therefore not in that
//! store by default — hence the handshake failure. Rather than requiring
//! every operator to run `certutil` manually (or to hand-edit forge-web's
//! `ca_cert_path`), we install the CA ourselves on every server boot.
//!
//! Strategy: shell out to `certutil.exe`. It's shipped with every modern
//! Windows (no new deps), handles cert-store bookkeeping correctly, and is
//! idempotent with `-f` (a repeat run replaces an existing entry with the
//! same bytes — a no-op in steady state, and cheap).
//!
//! Fallback order:
//!   1. `LocalMachine\Root` — machine-wide trust. This is what we want when
//!      running under LocalSystem as a Windows service. Requires admin.
//!   2. `CurrentUser\Root` — falls back to the running user's store when the
//!      machine-wide install is rejected (interactive, non-admin shell).
//!      Still covers forge-web when it runs as the same user.
//!
//! Failure is logged at WARN but never propagated — a permissions hiccup
//! must not keep the service from coming up.

#![cfg(windows)]

use std::path::Path;
use std::process::Command;

use tracing::{info, warn};

/// Subjects used by [`crate::tls_autogen::mint_ca`] — the current name
/// **and** any legacy names from previous releases — so purges cover
/// stale entries even after a subject rename.
const CA_SUBJECTS: &[&str] = &[
    "Forge VCS Local CA",    // current (rich DN with O + OU)
    "forge-server local CA", // legacy (pre-0.2)
];

/// Best-effort install. Never returns an error.
pub fn ensure_ca_trusted(ca_path: &Path) {
    if !ca_path.exists() {
        // No CA file to install (e.g. TLS disabled, or operator supplied
        // only a leaf with no separate CA). Nothing to do.
        return;
    }

    // Scrub stale copies FIRST. Repeat runs — or switching between data
    // dirs — create fresh CAs with the same CN as prior keys, and rustls
    // on the client side can end up picking the stale trust anchor and
    // fail leaf verification with `BadSignature`. Deleting all matching
    // entries up-front guarantees the only CA under that name is the one
    // we're about to write.
    purge_stale_ca(false);
    purge_stale_ca(true);

    let path_str = ca_path.to_string_lossy().to_string();

    if run_certutil(&["-addstore", "-f", "Root", &path_str]) {
        info!(
            "TLS CA installed in Windows LocalMachine\\Root: {}",
            ca_path.display()
        );
        return;
    }

    if run_certutil(&["-user", "-addstore", "-f", "Root", &path_str]) {
        warn!(
            "TLS CA installed in Windows CurrentUser\\Root (no admin — only the \
             current user will trust it): {}",
            ca_path.display()
        );
        return;
    }

    warn!(
        "Failed to auto-install TLS CA into the Windows trust store. \
         Clients will see 'BadSignature' until {} is trusted manually \
         (e.g. run `certutil -addstore -f Root <path>` from an elevated \
         prompt, or import via the MMC certificates snap-in).",
        ca_path.display()
    );
}

/// Remove every cert matching any subject in [`CA_SUBJECTS`] from the
/// Root store. `certutil -delstore` only removes one match per
/// invocation, so we retry in a bounded loop until it reports no more
/// hits (or we hit the retry cap as a safety net against certutil edge
/// cases). We loop over every known subject so an install after a CN
/// rename cleans up both the old and new entries.
fn purge_stale_ca(user_store: bool) {
    for subject in CA_SUBJECTS {
        for _ in 0..8 {
            let mut args: Vec<&str> = Vec::new();
            if user_store {
                args.push("-user");
            }
            args.extend_from_slice(&["-delstore", "Root", subject]);
            if !run_certutil_quiet(&args) {
                break; // No more matches for this subject.
            }
        }
    }
}

fn run_certutil(args: &[&str]) -> bool {
    match Command::new("certutil").args(args).output() {
        Ok(out) => out.status.success(),
        Err(e) => {
            warn!("certutil invocation failed: {e}");
            false
        }
    }
}

/// Like [`run_certutil`] but swallows the "nothing to delete" warning so
/// the log doesn't flood during purge.
fn run_certutil_quiet(args: &[&str]) -> bool {
    Command::new("certutil")
        .args(args)
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}
