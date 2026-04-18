// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! FFI-friendly wrappers around the CLI's `run` entry points.
//!
//! Each `ops::*` function matches one of the CLI subcommands but:
//!
//! - Accepts explicit context (workspace dir, CWD, flags) instead of
//!   `std::env::current_dir()` — so forge-ffi can drive the op from
//!   the UE editor's process without depending on the process-wide CWD.
//! - Returns a typed report (`AddReport`, `CommitReport`, …) rather
//!   than printing to stdout. The UI-facing `commands::*::run`
//!   continues to wrap these.
//!
//! The CLI `run` paths will migrate to call through this module in a
//! follow-up so there's exactly one implementation; for now these
//! functions shell out to the CLI `run` when the signature allows it,
//! or redirect CWD + invoke.

use std::path::{Path, PathBuf};

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
/// The plugin's `FForgeCheckInWorker` stages one file per invocation,
/// so this wraps the CLI `run` with a scoped CWD switch rather than
/// re-implementing the 200-line chunk + compress + index path. A
/// follow-up refactor will lift the core into a pure function and
/// drop the CWD dance.
pub fn add(workspace_root: &Path, paths: &[String]) -> Result<AddReport> {
    with_cwd(workspace_root, || {
        crate::commands::add::run(paths.to_vec())?;
        Ok(AddReport {
            // The CLI doesn't hand back a structured report today —
            // the plugin reads `forge status --json` afterwards. We
            // still return an empty success report so the FFI layer
            // has a stable shape to marshal.
            ..Default::default()
        })
    })
}

/// Run `forge commit -m <message>` against an explicit workspace.
pub fn commit(workspace_root: &Path, message: &str) -> Result<CommitReport> {
    with_cwd(workspace_root, || {
        // The current CLI implementation prints the commit hash to
        // stdout. Capture via the same re-run-status approach as
        // above; a follow-up extraction hands the hash back directly.
        // `commit` is backed by the `snapshot` module internally.
        crate::commands::snapshot::run(Some(message.to_string()), false, false, false)?;
        Ok(CommitReport {
            commit_hash: String::new(),
            message: message.to_string(),
            staged_count: 0,
        })
    })
}

/// Run `forge push` against an explicit workspace. `force` maps to
/// the `--force` flag.
pub fn push(workspace_root: &Path, force: bool) -> Result<PushReport> {
    with_cwd(workspace_root, || {
        crate::commands::push::run(force, None, None)?;
        Ok(PushReport::default())
    })
}

/// Run `forge pull` against an explicit workspace.
pub fn pull(workspace_root: &Path) -> Result<PullReport> {
    with_cwd(workspace_root, || {
        crate::commands::pull::run()?;
        Ok(PullReport::default())
    })
}

/// Scoped CWD switch. Saves + restores via a drop guard so a panic in
/// the body doesn't leak the changed CWD to other callers. Used
/// because the CLI `run` functions all call `std::env::current_dir()`
/// internally; refactoring every one to take an explicit workspace is
/// scope for a future slice.
fn with_cwd<F, T>(dir: &Path, f: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    struct Guard(Option<PathBuf>);
    impl Drop for Guard {
        fn drop(&mut self) {
            if let Some(prev) = self.0.take() {
                let _ = std::env::set_current_dir(prev);
            }
        }
    }

    let prev = std::env::current_dir().ok();
    std::env::set_current_dir(dir)?;
    let _guard = Guard(prev);
    f()
}
