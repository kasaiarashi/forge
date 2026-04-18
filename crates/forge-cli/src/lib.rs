// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Library face of the `forge` CLI.
//!
//! Exists so `forge-ffi` (the Unreal Engine bridge) can reuse the same
//! add/commit/push/pull implementations the CLI runs instead of forking
//! them into a parallel codebase. The `forge` binary still lives in
//! `main.rs` and links the same modules via the usual binary-crate
//! module tree.
//!
//! Public surface is deliberately narrow: only the `commands::*::run`
//! (or equivalent) entry points that FFI consumers need. Internal
//! helpers stay private to their modules.

use std::cell::RefCell;

pub mod commands;
pub mod pager;

// The shared client surface lives in the `forge-client` crate; re-
// export here so in-crate imports (`crate::client::...`) continue to
// resolve whether the binary or the library is compiling.
pub(crate) use forge_client::{client, credentials, tofu, url_resolver};

// Thread-local "current command server URL hint". Commands that know
// the target server up-front (`forge clone <url>`) stash it here so
// that if auth fails mid-execution, `offer_login` prompts with the
// correct URL instead of asking the user to re-type it. Lives in
// `lib.rs` (not `main.rs`) so both the binary and the library crate
// resolve `crate::set_server_url_hint(...)` to the same storage.
thread_local! {
    static SERVER_URL_HINT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Set the current command's server URL hint. Called by commands that
/// carry an explicit URL argument (e.g. `clone`).
pub fn set_server_url_hint(url: impl Into<String>) {
    SERVER_URL_HINT.with(|h| *h.borrow_mut() = Some(url.into()));
}

/// Read the hint. `None` when no command stashed one.
pub fn server_url_hint() -> Option<String> {
    SERVER_URL_HINT.with(|h| h.borrow().clone())
}

// FFI-friendly helpers. These are the entry points forge-ffi calls;
// they match the CLI's `run()` semantics but return typed
// `Result<ReportStruct, anyhow::Error>` rather than printing to stdout.
pub mod ops;
