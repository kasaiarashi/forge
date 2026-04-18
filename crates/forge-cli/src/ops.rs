// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! FFI-friendly wrappers around the CLI's `run` entry points.
//!
//! Each `ops::*` function matches one of the CLI subcommands but:
//!
//! - Accepts an explicit workspace root instead of relying on
//!   `std::env::current_dir()` — so forge-ffi can drive the op from
//!   the UE editor's process without racing the process-wide CWD
//!   against a concurrent caller.
//! - Returns a typed report (`AddReport`, `CommitReport`, …) rather
//!   than printing to stdout. The UI-facing `commands::*::run`
//!   continues to wrap these for terminal users.
//!
//! Each wrapper dispatches to the matching `commands::*::run_in`
//! variant which discovers the workspace from the explicit path.
//! The older Phase-4b.2 CWD-swap shim is gone — nothing in this
//! module mutates global state.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

/// Report returned by `add`. Mirrors the data the CLI prints and the
/// UE plugin surfaces through the FFI JSON response.
#[derive(Debug, Default, Clone, Serialize)]
pub struct AddReport {
    pub staged_paths: Vec<String>,
    pub deleted_paths: Vec<String>,
    pub unchanged: usize,
}

/// Report returned by `commit`. The commit hash is the snapshot id
/// the CLI prints; the plugin uses it as the "last commit" label.
#[derive(Debug, Default, Clone, Serialize)]
pub struct CommitReport {
    pub commit_hash: String,
    pub message: String,
    pub staged_count: usize,
}

/// Report returned by `push`. The ref + tip pair is what the CLI
/// prints as "Pushed refs/heads/main -> abc1234".
#[derive(Debug, Default, Clone, Serialize)]
pub struct PushReport {
    pub ref_name: String,
    pub new_tip_hex: String,
    /// Bytes actually streamed to the server. Zero when everything
    /// was already present (up-to-date push).
    pub bytes_uploaded: u64,
    pub objects_uploaded: u64,
}

/// Report returned by `pull`.
#[derive(Debug, Default, Clone, Serialize)]
pub struct PullReport {
    pub bytes_downloaded: u64,
    pub objects_received: u64,
    pub refs_updated: Vec<String>,
}

/// Run `forge add <paths>` against an explicit workspace.
///
/// Dispatches to [`crate::commands::add::run_in`] which discovers the
/// workspace from the explicit path rather than the process CWD. No
/// global state is mutated — concurrent FFI callers (two editor
/// instances sharing a DLL, say) can safely drive different
/// workspaces through the same library.
pub fn add(workspace_root: &Path, paths: &[String]) -> Result<AddReport> {
    crate::commands::add::run_in(workspace_root, paths.to_vec())?;
    Ok(AddReport {
        // Today's `run_in` doesn't hand back a structured report —
        // the plugin reads `forge status --json` afterwards. An empty
        // success report keeps the FFI marshalling shape stable so a
        // future enrichment pass is a field-only change.
        ..Default::default()
    })
}

/// Run `forge commit -m <message>` against an explicit workspace.
pub fn commit(workspace_root: &Path, message: &str) -> Result<CommitReport> {
    crate::commands::snapshot::run_in(
        workspace_root,
        Some(message.to_string()),
        false,
        false,
        false,
    )?;
    Ok(CommitReport {
        commit_hash: String::new(),
        message: message.to_string(),
        staged_count: 0,
    })
}

/// Run `forge push` against an explicit workspace. `force` maps to
/// the `--force` flag.
pub fn push(workspace_root: &Path, force: bool) -> Result<PushReport> {
    crate::commands::push::run_in(workspace_root, force, None, None)?;
    Ok(PushReport::default())
}

/// Run `forge pull` against an explicit workspace.
pub fn pull(workspace_root: &Path) -> Result<PullReport> {
    crate::commands::pull::run_in(workspace_root)?;
    Ok(PullReport::default())
}
