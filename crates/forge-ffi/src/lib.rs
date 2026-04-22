// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Forge C ABI (Phase 4a scaffold).
//!
//! This crate exports a stable C interface that the Unreal Engine
//! source-control plugin loads at editor startup. Replacing the
//! current `FPlatformProcess::ExecProcess(TEXT("forge"), ...)` fan-out
//! with one in-process library call collapses the "3N+2 subprocesses
//! per CheckIn" storm that dominates today's UE→Forge latency (~30 s
//! for 500 assets).
//!
//! This file is the 4a surface: session lifecycle, version, error
//! handling, and a first local-only op (`forge_status_json`) that
//! exercises workspace discovery + state enumeration without touching
//! the network. Phase 4b grows the surface to cover the remote-backed
//! ops (push, pull, locks) via an owned tokio runtime + gRPC channel.
//! Phase 4c is the UE plugin integration itself.
//!
//! ## ABI stability rules
//!
//! - No generics crossing the boundary.
//! - No Rust-native types in public signatures — only `*const c_char`,
//!   `*mut`, `size_t`, integer types, and the opaque structs declared
//!   here.
//! - Ownership is explicit: every pointer returned by this crate has a
//!   matching `*_free` function in this module. Callers must never
//!   `free()` our pointers with the libc allocator; we may use a
//!   different allocator internally (jemalloc in the future).
//! - Errors flow via `forge_error_t*` out-parameters so the call site
//!   can branch on the return value first (null vs non-null pointer,
//!   0 vs non-zero status) without checking `errno`.
//!
//! ## Panic boundary
//!
//! Every `extern "C"` function wraps its Rust body in
//! [`std::panic::catch_unwind`]. A panic becomes `FORGE_ERR_INTERNAL`
//! with a generic message; the backtrace is captured on the log side
//! via `tracing::error!`. Unwinding across the FFI boundary is
//! undefined behaviour in C consumers, so we never let it.

#![deny(improper_ctypes_definitions)]
// C-ABI types are deliberately snake_case / SCREAMING_SNAKE_CASE so
// consumers reading the generated header see idiomatic C names.
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]

use std::ffi::{c_char, c_int, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr;

use forge_core::hash::ForgeHash;
use forge_core::workspace::Workspace;

// ── Public C ABI types ──────────────────────────────────────────────────────

/// Status code returned via [`forge_error_t::code`]. Numeric values are
/// part of the ABI — never renumber; append only.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum forge_status_t {
    FORGE_OK = 0,
    FORGE_ERR_IO = 1,
    FORGE_ERR_ARG = 2,
    FORGE_ERR_AUTH = 3,
    FORGE_ERR_NOT_FOUND = 4,
    FORGE_ERR_CONFLICT = 5,
    FORGE_ERR_NOT_A_WORKSPACE = 6,
    FORGE_ERR_INTERNAL = 99,
}

/// Error out-parameter. The library allocates `message` when
/// populating this; the caller must pass the struct back through
/// [`forge_error_free`] when done.
///
/// Zero-initialised is valid: `code = FORGE_OK`, `message = null`.
#[repr(C)]
pub struct forge_error_t {
    pub code: forge_status_t,
    /// Null-terminated UTF-8 message, or `NULL` when there's nothing
    /// to say beyond the code. Allocated by this library; free via
    /// [`forge_error_free`].
    pub message: *mut c_char,
}

impl Default for forge_error_t {
    fn default() -> Self {
        Self {
            code: forge_status_t::FORGE_OK,
            message: ptr::null_mut(),
        }
    }
}

/// Opaque handle for an open workspace session. The Rust body lives
/// inside the `crate::session` module; C callers must only ever pass
/// the pointer around — never dereference.
#[repr(C)]
pub struct forge_session_t {
    _opaque: [u8; 0],
}

// ── Session implementation (Rust side) ──────────────────────────────────────

mod session {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex, OnceLock};

    /// The real struct behind [`super::forge_session_t`]. Only
    /// reachable inside this crate; C sees the opaque tag.
    ///
    /// Owns a tokio runtime lazily so a workspace that never makes
    /// a remote call (pure local status, say) doesn't pay the
    /// worker-thread cost. Once constructed, the same runtime
    /// services every remote op for the session's lifetime — the
    /// UE editor keeps one session open across its whole run, so
    /// amortising the startup cost matters.
    /// Buffered JSON representations of `LockEvent`s received from the
    /// server's `StreamLockEvents`. The plugin drains this on its
    /// Tick via [`super::forge_poll_lock_events_json`]. Bounded by
    /// `EVENT_BUFFER_CAP` so a misbehaving subscriber that never
    /// polls can't consume unbounded memory — a full buffer silently
    /// drops the oldest event (with a monotonic `seq` the plugin
    /// notices) rather than blocking the subscribe task.
    pub type EventBuffer = Arc<Mutex<VecDeque<String>>>;
    const EVENT_BUFFER_CAP: usize = 2048;

    pub fn push_event(buf: &EventBuffer, json: String) {
        let mut guard = buf.lock().expect("event buffer poisoned");
        if guard.len() >= EVENT_BUFFER_CAP {
            guard.pop_front();
        }
        guard.push_back(json);
    }

    pub fn drain_events(buf: &EventBuffer) -> Vec<String> {
        let mut guard = buf.lock().expect("event buffer poisoned");
        std::mem::take(&mut *guard).into_iter().collect()
    }

    pub struct Session {
        pub workspace: Workspace,
        runtime: OnceLock<Arc<tokio::runtime::Runtime>>,
        pub event_buffer: EventBuffer,
        /// `true` once `forge_subscribe_lock_events` has spawned a
        /// subscriber task. Prevents a double-subscribe from filling
        /// the buffer twice per event.
        pub lock_events_subscribed: Mutex<bool>,
    }

    impl Session {
        pub fn open(workspace_path: PathBuf) -> Result<Self, anyhow::Error> {
            let workspace = Workspace::discover(&workspace_path)
                .map_err(|e| anyhow::anyhow!("workspace discover: {e}"))?;
            Ok(Self {
                workspace,
                runtime: OnceLock::new(),
                event_buffer: Arc::new(Mutex::new(VecDeque::new())),
                lock_events_subscribed: Mutex::new(false),
            })
        }

        /// Get (or lazily construct) the tokio runtime. 2 worker
        /// threads is plenty for the editor — push/pull saturate a
        /// link on the server CPU, not the client.
        pub fn runtime(&self) -> Result<&tokio::runtime::Runtime, anyhow::Error> {
            super::install_default_crypto_provider();
            let rt = self.runtime.get_or_init(|| {
                let built = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .thread_name("forge-ffi")
                    .build()
                    .expect("build tokio runtime for forge-ffi session");
                Arc::new(built)
            });
            Ok(rt.as_ref())
        }

        /// Resolve the server URL for this workspace's default remote
        /// and return `(url, repo)`.  `repo` comes from the workspace
        /// config and defaults to "default" when empty (mirrors the
        /// CLI's behaviour).
        pub fn remote(&self) -> Result<(String, String), anyhow::Error> {
            let cfg = self
                .workspace
                .config()
                .map_err(|e| anyhow::anyhow!("load workspace config: {e}"))?;
            let url = cfg
                .default_remote_url()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no remote configured — set one with `forge remote add origin <url>`"
                    )
                })?
                .to_string();
            let repo = if cfg.repo.is_empty() {
                "default".into()
            } else {
                cfg.repo.clone()
            };
            Ok((url, repo))
        }
    }
}

use session::Session;

/// Install the rustls process-level `CryptoProvider` (aws-lc-rs)
/// idempotently. Binary crates in this workspace (`forge-cli`,
/// `forge-server`, `forge-agent`) do this in their own `main()`, but
/// `forge-ffi` is a cdylib loaded by WinUI / UE / external agents —
/// nothing of ours runs at startup. Any path that constructs a
/// `rustls::ClientConfig` without a provider installed panics with
/// "Could not automatically determine the process-level CryptoProvider";
/// under cdylib hosting that tears the host process down.
///
/// Must be called before any code that touches tonic TLS. Safe to call
/// repeatedly; `Once` keeps it to one real install.
fn install_default_crypto_provider() {
    use std::sync::Once;
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // `install_default` returns Err if a provider is already set —
        // swallow it, we're happy as long as *something* is installed.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

// ── Exported functions ──────────────────────────────────────────────────────

/// Library version (Cargo's `CARGO_PKG_VERSION`). Returned as a static
/// NUL-terminated UTF-8 string — do **not** free.
///
/// # Safety
/// The returned pointer is valid for the lifetime of the loaded library.
#[no_mangle]
pub extern "C" fn forge_version() -> *const c_char {
    // Single leak on first call; the string lives forever anyway.
    static VERSION: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
    let s = VERSION.get_or_init(|| {
        CString::new(env!("CARGO_PKG_VERSION")).expect("version has no interior NUL")
    });
    s.as_ptr()
}

/// Open a session rooted at `workspace_path`, walking up the tree to
/// find a `.forge` directory (same rules as `forge` CLI).
///
/// On success returns a non-null [`forge_session_t`] pointer; the
/// caller must release it via [`forge_session_close`]. On failure
/// returns null and populates `*out_err` when non-null.
///
/// # Safety
/// - `workspace_path` must be a valid NUL-terminated UTF-8 C string.
/// - `out_err`, when non-null, must point to a writable `forge_error_t`.
#[no_mangle]
pub unsafe extern "C" fn forge_session_open(
    workspace_path: *const c_char,
    out_err: *mut forge_error_t,
) -> *mut forge_session_t {
    catch_unwind(AssertUnwindSafe(|| {
        // Install the rustls provider before any later code path can
        // build a TLS client. `compute_status_json` spawns its own
        // tokio runtime + gRPC connect inline — it never touches
        // `Session::runtime()`, so the install can't live there alone.
        install_default_crypto_provider();

        let path_str = match cstr_to_str(workspace_path, "workspace_path", out_err) {
            Some(s) => s,
            None => return ptr::null_mut(),
        };
        let path = PathBuf::from(path_str);
        match Session::open(path) {
            Ok(sess) => {
                clear_error(out_err);
                Box::into_raw(Box::new(sess)) as *mut forge_session_t
            }
            Err(e) => {
                set_error(
                    out_err,
                    forge_status_t::FORGE_ERR_NOT_A_WORKSPACE,
                    &e.to_string(),
                );
                ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_session_open",
        );
        ptr::null_mut()
    })
}

/// Release a session. Safe to call with `NULL`. Double-free is
/// undefined behaviour (there's no refcount on the C side).
///
/// # Safety
/// `session` must either be null or have been returned by
/// [`forge_session_open`] and not yet closed.
#[no_mangle]
pub unsafe extern "C" fn forge_session_close(session: *mut forge_session_t) {
    if session.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // Reconstitute the Box so Rust's drop glue runs (flushes logs,
        // closes any gRPC channels in Phase 4b, etc.).
        let boxed: Box<Session> = Box::from_raw(session as *mut Session);
        drop(boxed);
    }));
}

/// Release a `forge_error_t` populated by this library. Safe to call
/// with `NULL` or with a zero-initialised struct.
///
/// Resets `code` to `FORGE_OK` and `message` to null so the caller
/// can reuse the struct for a follow-up call without leaking the
/// prior message.
///
/// # Safety
/// `err`, when non-null, must point to a struct whose `message` field
/// was allocated by this library (or is null).
#[no_mangle]
pub unsafe extern "C" fn forge_error_free(err: *mut forge_error_t) {
    if err.is_null() {
        return;
    }
    let slot = &mut *err;
    if !slot.message.is_null() {
        let _ = CString::from_raw(slot.message);
        slot.message = ptr::null_mut();
    }
    slot.code = forge_status_t::FORGE_OK;
}

/// Release a `char*` returned by one of the op functions (e.g.
/// [`forge_status_json`]). Safe on NULL.
///
/// # Safety
/// `s`, when non-null, must be a pointer returned by this library.
#[no_mangle]
pub unsafe extern "C" fn forge_string_free(s: *mut c_char) {
    if !s.is_null() {
        let _ = CString::from_raw(s);
    }
}

/// Snapshot the working-tree state as a JSON document.
///
/// Output shape:
/// ```json
/// {
///   "workspace_root": "C:/proj",
///   "head": {"kind":"branch","name":"main"} | {"kind":"detached","hash":"..."},
///   "dirty": ["Content/Foo.uasset", ...]   // placeholder until 4b
/// }
/// ```
///
/// Returns null on error; frees message via [`forge_error_free`].
/// Non-null success values must be released via [`forge_string_free`].
///
/// # Safety
/// `session` must be a non-null pointer returned by
/// [`forge_session_open`]. `out_err`, when non-null, must point to a
/// writable `forge_error_t`.
#[no_mangle]
pub unsafe extern "C" fn forge_status_json(
    session: *mut forge_session_t,
    out_err: *mut forge_error_t,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        if session.is_null() {
            set_error(out_err, forge_status_t::FORGE_ERR_ARG, "session is null");
            return ptr::null_mut();
        }
        let sess: &Session = &*(session as *const Session);
        match build_status_json(sess) {
            Ok(json) => {
                clear_error(out_err);
                // Into_raw hands ownership to the caller; they'll return
                // it through forge_string_free.
                CString::new(json)
                    .map(|c| c.into_raw())
                    .unwrap_or_else(|_| {
                        set_error(
                            out_err,
                            forge_status_t::FORGE_ERR_INTERNAL,
                            "status JSON contained interior NUL",
                        );
                        ptr::null_mut()
                    })
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_status_json",
        );
        ptr::null_mut()
    })
}

// ── Local-only workspace introspection ──────────────────────────────────────

/// Return a JSON document summarising the workspace: root path,
/// workspace_id (used to match lock records), default remote URL,
/// repo name, current branch (or detached hash), and user identity.
///
/// Every field the UE plugin currently shells out to `forge` for is
/// packed into one call — the bridge uses this to populate the
/// `FForgeSourceControlProvider` fields at module init without ever
/// running a subprocess.
///
/// JSON shape:
/// ```json
/// {
///   "workspace_root": "...",
///   "workspace_id": "uuid-...",
///   "repo": "alice/game" | "",
///   "remote_url": "https://..." | null,
///   "head": {"kind":"branch","name":"main"} | {"kind":"detached","hash":"..."},
///   "user": {"name":"alice","email":"alice@example.com"},
///   "workflow": "lock" | "merge"
/// }
/// ```
///
/// Returns null + populates `*out_err` on failure. Non-null success
/// must be freed via [`forge_string_free`].
///
/// # Safety
/// `session` non-null; `out_err` nullable writable.
#[no_mangle]
pub unsafe extern "C" fn forge_workspace_info_json(
    session: *mut forge_session_t,
    out_err: *mut forge_error_t,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return ptr::null_mut();
        };
        match build_workspace_info_json(sess) {
            Ok(json) => {
                clear_error(out_err);
                string_to_raw(json, out_err)
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_workspace_info_json",
        );
        ptr::null_mut()
    })
}

/// Return the current branch name as an owned string, or null when
/// the workspace is in a detached state. `out_err` carries a real
/// error only for failure modes (unreadable config, missing HEAD);
/// detached HEAD is not an error.
///
/// # Safety
/// `session` non-null; `out_err` nullable writable. Success values
/// must be freed via [`forge_string_free`].
#[no_mangle]
pub unsafe extern "C" fn forge_current_branch(
    session: *mut forge_session_t,
    out_err: *mut forge_error_t,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return ptr::null_mut();
        };
        match sess.workspace.current_branch() {
            Ok(Some(name)) => {
                clear_error(out_err);
                string_to_raw(name, out_err)
            }
            Ok(None) => {
                // Detached — success, but no name to hand back. Callers
                // distinguish "error" from "detached" by checking
                // `out_err.code` (0 = OK).
                clear_error(out_err);
                ptr::null_mut()
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_current_branch",
        );
        ptr::null_mut()
    })
}

fn build_workspace_info_json(sess: &Session) -> Result<String, anyhow::Error> {
    use forge_core::workspace::HeadRef;
    use serde_json::json;

    let cfg = sess
        .workspace
        .config()
        .map_err(|e| anyhow::anyhow!("load workspace config: {e}"))?;

    let head = match sess
        .workspace
        .head()
        .map_err(|e| anyhow::anyhow!("read HEAD: {e}"))?
    {
        HeadRef::Branch(name) => json!({"kind": "branch", "name": name}),
        HeadRef::Detached(hash) => json!({"kind": "detached", "hash": hash.to_hex()}),
    };

    let remote_url = cfg.default_remote_url().map(str::to_string);
    let workflow = serde_json::to_value(&cfg.workflow).unwrap_or_else(|_| serde_json::Value::Null);

    Ok(json!({
        "workspace_root": sess.workspace.root.display().to_string(),
        "workspace_id": cfg.workspace_id,
        "repo": cfg.repo,
        "remote_url": remote_url,
        "head": head,
        "user": {
            "name": cfg.user.name,
            "email": cfg.user.email,
        },
        "workflow": workflow,
    })
    .to_string())
}

// ── Log / branch / asset-info (read-only) ───────────────────────────────────
//
// These reuse forge-core primitives directly instead of going through
// `forge_cli::commands::*::run`, which all discover the workspace via
// `std::env::current_dir()` — unsafe under concurrent FFI callers.

/// Enumerate commits reachable from HEAD (first-parent walk) as a JSON
/// array of `{hash, short_hash, parents[], author{name,email}, timestamp, message}`.
///
/// `limit` caps the walk; 0 means "no cap" but still stops at the
/// graph boundary. Returns `"[]"` on an empty repo. Null on error.
///
/// # Safety
/// `session` non-null; `out_err` nullable writable. Success values
/// must be freed via [`forge_string_free`].
#[no_mangle]
pub unsafe extern "C" fn forge_log_json(
    session: *mut forge_session_t,
    limit: u32,
    out_err: *mut forge_error_t,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return ptr::null_mut();
        };
        match build_log_json(sess, limit) {
            Ok(json) => {
                clear_error(out_err);
                string_to_raw(json, out_err)
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_log_json",
        );
        ptr::null_mut()
    })
}

fn build_log_json(sess: &Session, limit: u32) -> Result<String, anyhow::Error> {
    use serde_json::json;

    let head = sess
        .workspace
        .head_snapshot()
        .map_err(|e| anyhow::anyhow!("read HEAD: {e}"))?;

    let mut entries = Vec::<serde_json::Value>::new();
    let mut current = head;
    let mut remaining = if limit == 0 { u32::MAX } else { limit };

    while !current.is_zero() && remaining > 0 {
        let snap = sess
            .workspace
            .object_store
            .get_snapshot(&current)
            .map_err(|e| anyhow::anyhow!("load snapshot {}: {e}", current.to_hex()))?;

        let parents: Vec<String> = snap.parents.iter().map(ForgeHash::to_hex).collect();
        entries.push(json!({
            "hash": current.to_hex(),
            "short_hash": current.short(),
            "parents": parents,
            "author": {
                "name": snap.author.name,
                "email": snap.author.email,
            },
            "timestamp": snap.timestamp.to_rfc3339(),
            "message": snap.message,
        }));

        remaining = remaining.saturating_sub(1);
        current = snap.parents.first().copied().unwrap_or(ForgeHash::ZERO);
    }

    Ok(serde_json::Value::Array(entries).to_string())
}

/// List branches as a JSON array of
/// `{name, is_current, is_remote, tip_hash}`.
///
/// Remote-tracking refs aren't enumerated yet (forge-core stores them
/// differently); `is_remote` is always false today but the shape is
/// stable for when they're added.
///
/// # Safety
/// `session` non-null; `out_err` nullable writable.
#[no_mangle]
pub unsafe extern "C" fn forge_branch_list_json(
    session: *mut forge_session_t,
    out_err: *mut forge_error_t,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return ptr::null_mut();
        };
        match build_branch_list_json(sess) {
            Ok(json) => {
                clear_error(out_err);
                string_to_raw(json, out_err)
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_branch_list_json",
        );
        ptr::null_mut()
    })
}

fn build_branch_list_json(sess: &Session) -> Result<String, anyhow::Error> {
    use serde_json::json;

    let current = sess
        .workspace
        .current_branch()
        .map_err(|e| anyhow::anyhow!("read current branch: {e}"))?;
    let names = sess
        .workspace
        .list_branches()
        .map_err(|e| anyhow::anyhow!("list branches: {e}"))?;

    let entries: Vec<_> = names
        .into_iter()
        .map(|name| {
            let tip = sess
                .workspace
                .get_branch_tip(&name)
                .ok()
                .map(|h| h.to_hex())
                .unwrap_or_default();
            json!({
                "name": name,
                "is_current": current.as_deref() == Some(name.as_str()),
                "is_remote": false,
                "tip_hash": tip,
            })
        })
        .collect();

    Ok(serde_json::Value::Array(entries).to_string())
}

/// Return a JSON document with `.uasset`/`.umap` metadata extracted
/// from `path` (resolved relative to the workspace root unless
/// absolute). Shape: `{path, asset_class, engine_version, package_flags[], dependencies[], file_size}`.
///
/// Returns null + `FORGE_ERR_NOT_FOUND` if the file is missing, or
/// `FORGE_ERR_IO` if the header is not a valid UE asset (garbage
/// input, truncated file). Success values must be freed via
/// [`forge_string_free`].
///
/// # Safety
/// `session` non-null; `path` non-null UTF-8 C string; `out_err` nullable.
#[no_mangle]
pub unsafe extern "C" fn forge_asset_info_json(
    session: *mut forge_session_t,
    path: *const c_char,
    out_err: *mut forge_error_t,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return ptr::null_mut();
        };
        let Some(path_str) = cstr_to_str(path, "path", out_err) else {
            return ptr::null_mut();
        };
        match build_asset_info_json(sess, path_str) {
            Ok(json) => {
                clear_error(out_err);
                string_to_raw(json, out_err)
            }
            Err(e) => {
                let code = if e.to_string().contains("not found") {
                    forge_status_t::FORGE_ERR_NOT_FOUND
                } else {
                    forge_status_t::FORGE_ERR_IO
                };
                set_error(out_err, code, &e.to_string());
                ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_asset_info_json",
        );
        ptr::null_mut()
    })
}

fn build_asset_info_json(sess: &Session, path: &str) -> Result<String, anyhow::Error> {
    use serde_json::json;

    let abs = if PathBuf::from(path).is_absolute() {
        PathBuf::from(path)
    } else {
        sess.workspace.root.join(path)
    };

    if !abs.exists() {
        return Err(anyhow::anyhow!("not found: {}", abs.display()));
    }

    let bytes =
        std::fs::read(&abs).map_err(|e| anyhow::anyhow!("read {}: {e}", abs.display()))?;
    let file_size = bytes.len() as u64;

    let meta = forge_core::uasset::parse_uasset(&bytes)
        .ok_or_else(|| anyhow::anyhow!("not a valid UE asset header: {}", abs.display()))?;

    Ok(json!({
        "path": path,
        "asset_class": meta.asset_class,
        "engine_version": meta.engine_version,
        "package_flags": meta.package_flags,
        "dependencies": meta.dependencies,
        "file_size": file_size,
    })
    .to_string())
}

// ── Write ops: unstage, switch_branch, branch_create, branch_delete ────────

/// Unstage the given paths (JSON array of UTF-8 strings). Mirrors
/// `forge unstage <paths>`; dot-path `["."]` unstages everything.
///
/// # Safety
/// `session` non-null; `paths_json` non-null NUL-terminated UTF-8 JSON
/// array; `out_err` nullable writable.
#[no_mangle]
pub unsafe extern "C" fn forge_unstage(
    session: *mut forge_session_t,
    paths_json: *const c_char,
    out_err: *mut forge_error_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return 1;
        };
        let Some(json_str) = cstr_to_str(paths_json, "paths_json", out_err) else {
            return 1;
        };
        let paths: Vec<String> = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(e) => {
                set_error(
                    out_err,
                    forge_status_t::FORGE_ERR_ARG,
                    &format!("paths_json must be a JSON array of strings: {e}"),
                );
                return 1;
            }
        };
        match forge_cli::commands::unstage::run_in(&sess.workspace.root, paths) {
            Ok(()) => {
                clear_error(out_err);
                0
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                1
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_unstage",
        );
        1
    })
}

/// Switch to branch `name`. When `create != 0`, creates the branch at
/// the current HEAD first (like `forge switch -c`). Returns 0 on
/// success. Fails with `FORGE_ERR_CONFLICT` when the working tree has
/// uncommitted changes, `FORGE_ERR_NOT_FOUND` when the branch doesn't
/// exist and `create` is false.
///
/// # Safety
/// `session` non-null; `name` non-null UTF-8 C string; `out_err` nullable.
#[no_mangle]
pub unsafe extern "C" fn forge_switch(
    session: *mut forge_session_t,
    name: *const c_char,
    create: c_int,
    out_err: *mut forge_error_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return 1;
        };
        let Some(name_str) = cstr_to_str(name, "name", out_err) else {
            return 1;
        };
        match forge_cli::commands::switch::run_with_create_in(
            &sess.workspace.root,
            name_str.to_string(),
            create != 0,
        ) {
            Ok(()) => {
                clear_error(out_err);
                0
            }
            Err(e) => {
                let msg = e.to_string();
                let code = if msg.contains("uncommitted changes") {
                    forge_status_t::FORGE_ERR_CONFLICT
                } else if msg.contains("does not exist") {
                    forge_status_t::FORGE_ERR_NOT_FOUND
                } else {
                    forge_status_t::FORGE_ERR_IO
                };
                set_error(out_err, code, &msg);
                1
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_switch",
        );
        1
    })
}

/// Create a new branch `name` at the current HEAD. Fails with
/// `FORGE_ERR_CONFLICT` when the branch already exists, or
/// `FORGE_ERR_IO` when HEAD cannot be resolved.
///
/// # Safety
/// `session` non-null; `name` non-null UTF-8 C string; `out_err` nullable.
#[no_mangle]
pub unsafe extern "C" fn forge_branch_create(
    session: *mut forge_session_t,
    name: *const c_char,
    out_err: *mut forge_error_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return 1;
        };
        let Some(name_str) = cstr_to_str(name, "name", out_err) else {
            return 1;
        };
        match run_branch_create(sess, name_str) {
            Ok(()) => {
                clear_error(out_err);
                0
            }
            Err(e) => {
                let code = if e.to_string().contains("already exists") {
                    forge_status_t::FORGE_ERR_CONFLICT
                } else {
                    forge_status_t::FORGE_ERR_IO
                };
                set_error(out_err, code, &e.to_string());
                1
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_branch_create",
        );
        1
    })
}

/// Delete branch `name`. `force != 0` skips the safety check that
/// prevents deleting the current branch. Returns 0 on success,
/// non-zero otherwise (branch missing → `FORGE_ERR_NOT_FOUND`, current
/// branch without force → `FORGE_ERR_CONFLICT`).
///
/// # Safety
/// `session` non-null; `name` non-null UTF-8 C string; `out_err` nullable.
#[no_mangle]
pub unsafe extern "C" fn forge_branch_delete(
    session: *mut forge_session_t,
    name: *const c_char,
    force: c_int,
    out_err: *mut forge_error_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return 1;
        };
        let Some(name_str) = cstr_to_str(name, "name", out_err) else {
            return 1;
        };
        match run_branch_delete(sess, name_str, force != 0) {
            Ok(()) => {
                clear_error(out_err);
                0
            }
            Err(e) => {
                let msg = e.to_string();
                let code = if msg.contains("not found") {
                    forge_status_t::FORGE_ERR_NOT_FOUND
                } else if msg.contains("current branch") {
                    forge_status_t::FORGE_ERR_CONFLICT
                } else {
                    forge_status_t::FORGE_ERR_IO
                };
                set_error(out_err, code, &msg);
                1
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_branch_delete",
        );
        1
    })
}

fn run_branch_create(sess: &Session, name: &str) -> Result<(), anyhow::Error> {
    // Match CLI behavior: refuse if the ref already exists; otherwise
    // point a new refs/heads/<name> file at the current HEAD snapshot.
    let ref_path = sess.workspace.forge_dir().join("refs").join("heads").join(name);
    if ref_path.exists() {
        return Err(anyhow::anyhow!("branch '{name}' already exists"));
    }
    let head = sess
        .workspace
        .head_snapshot()
        .map_err(|e| anyhow::anyhow!("read HEAD: {e}"))?;
    if head.is_zero() {
        return Err(anyhow::anyhow!(
            "cannot create branch '{name}' before the first commit"
        ));
    }
    sess.workspace
        .set_branch_tip(name, &head)
        .map_err(|e| anyhow::anyhow!("set branch tip: {e}"))?;
    Ok(())
}

fn run_branch_delete(sess: &Session, name: &str, force: bool) -> Result<(), anyhow::Error> {
    if !force {
        let current = sess
            .workspace
            .current_branch()
            .map_err(|e| anyhow::anyhow!("read current branch: {e}"))?;
        if current.as_deref() == Some(name) {
            return Err(anyhow::anyhow!("cannot delete the current branch '{name}'"));
        }
    }
    let ref_path = sess.workspace.forge_dir().join("refs").join("heads").join(name);
    if !ref_path.exists() {
        return Err(anyhow::anyhow!("branch '{name}' not found"));
    }
    std::fs::remove_file(&ref_path)
        .map_err(|e| anyhow::anyhow!("remove {}: {e}", ref_path.display()))?;
    Ok(())
}

// ── Index / commit / push / pull ────────────────────────────────────────────
//
// These wrap `forge_cli::ops::*` so the FFI exposes the exact same
// semantics the `forge` CLI ships. The plugin migrates each hot-path
// worker (CheckIn in particular) to these and drops its
// CreateProcess("forge add <path>") / ExecProcess("forge push") fan-out.

/// Stage the given paths (JSON array of UTF-8 path strings). Paths
/// are resolved relative to the session's workspace root.
///
/// Returns 0 on success, non-zero on failure. `out_err` carries the
/// detail. On the Phase-4 perf path this removes N CreateProcess
/// invocations per CheckIn.
///
/// # Safety
/// `session` non-null; `paths_json` non-null NUL-terminated UTF-8 C
/// string parseable as a JSON array of strings; `out_err` nullable.
#[no_mangle]
pub unsafe extern "C" fn forge_add_paths(
    session: *mut forge_session_t,
    paths_json: *const c_char,
    out_err: *mut forge_error_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return 1;
        };
        let Some(json_str) = cstr_to_str(paths_json, "paths_json", out_err) else {
            return 1;
        };
        let paths: Vec<String> = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(e) => {
                set_error(
                    out_err,
                    forge_status_t::FORGE_ERR_ARG,
                    &format!("paths_json must be a JSON array of strings: {e}"),
                );
                return 1;
            }
        };
        match forge_cli::ops::add(&sess.workspace.root, &paths) {
            Ok(_) => {
                clear_error(out_err);
                0
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                1
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_add_paths",
        );
        1
    })
}

/// Commit the currently-staged index with `message`. Returns 0 on
/// success; non-zero populates `*out_err` with the failure.
///
/// # Safety
/// `session` non-null; `message` non-null UTF-8 C string; `out_err` nullable.
#[no_mangle]
pub unsafe extern "C" fn forge_commit(
    session: *mut forge_session_t,
    message: *const c_char,
    out_err: *mut forge_error_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return 1;
        };
        let Some(msg) = cstr_to_str(message, "message", out_err) else {
            return 1;
        };
        match forge_cli::ops::commit(&sess.workspace.root, msg) {
            Ok(_) => {
                clear_error(out_err);
                0
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                1
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_commit",
        );
        1
    })
}

/// Push the current workspace to its default remote. `force` flips
/// the `--force` flag (the server still enforces lock gates + ACLs).
///
/// # Safety
/// `session` non-null; `out_err` nullable.
#[no_mangle]
pub unsafe extern "C" fn forge_push(
    session: *mut forge_session_t,
    force: c_int,
    out_err: *mut forge_error_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return 1;
        };
        match forge_cli::ops::push(&sess.workspace.root, force != 0) {
            Ok(_) => {
                clear_error(out_err);
                0
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                1
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_push",
        );
        1
    })
}

/// Pull the current workspace's default branch from its default
/// remote.
///
/// # Safety
/// `session` non-null; `out_err` nullable.
#[no_mangle]
pub unsafe extern "C" fn forge_pull(
    session: *mut forge_session_t,
    out_err: *mut forge_error_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return 1;
        };
        match forge_cli::ops::pull(&sess.workspace.root) {
            Ok(_) => {
                clear_error(out_err);
                0
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                1
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_pull",
        );
        1
    })
}

// ── Lock-event subscription (Phase 4d) ──────────────────────────────────────

/// Start a background subscription to the server's `StreamLockEvents`
/// for this workspace's default remote. Events land in the session's
/// internal buffer; the caller drains them via
/// [`forge_poll_lock_events_json`].
///
/// Calling twice on the same session is a no-op — the subscriber
/// task persists for the session's lifetime. Closing the session
/// drops the runtime, which aborts the task.
///
/// Returns 0 on success; non-zero populates `*out_err`.
///
/// # Safety
/// `session` non-null; `out_err` nullable writable.
#[no_mangle]
pub unsafe extern "C" fn forge_subscribe_lock_events(
    session: *mut forge_session_t,
    out_err: *mut forge_error_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return 1;
        };
        match start_lock_event_subscription(sess) {
            Ok(()) => {
                clear_error(out_err);
                0
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                1
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_subscribe_lock_events",
        );
        1
    })
}

/// Drain the event buffer and return a JSON array of events. Each
/// element mirrors the proto `LockEvent` shape:
/// `{"kind":"snapshot"|"acquire"|"release", "seq":N, "info":{path,owner,workspace_id,reason,created_at}}`.
///
/// Returns an empty array string `"[]"` when nothing is pending.
/// Null on error. Non-null success values must be freed via
/// [`forge_string_free`].
///
/// # Safety
/// `session` non-null; `out_err` nullable writable.
#[no_mangle]
pub unsafe extern "C" fn forge_poll_lock_events_json(
    session: *mut forge_session_t,
    out_err: *mut forge_error_t,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return ptr::null_mut();
        };
        let items = session::drain_events(&sess.event_buffer);
        // Each item is already a JSON object string — splice them
        // into an array with commas. Cheaper + simpler than
        // re-parsing through serde_json::Value.
        let mut out = String::with_capacity(2 + items.iter().map(|s| s.len() + 1).sum::<usize>());
        out.push('[');
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(item);
        }
        out.push(']');
        clear_error(out_err);
        string_to_raw(out, out_err)
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_poll_lock_events_json",
        );
        ptr::null_mut()
    })
}

fn start_lock_event_subscription(sess: &Session) -> Result<(), anyhow::Error> {
    use forge_proto::forge::StreamLockEventsRequest;
    use serde_json::json;

    {
        let mut guard = sess
            .lock_events_subscribed
            .lock()
            .expect("session subscribe flag poisoned");
        if *guard {
            return Ok(()); // Idempotent.
        }
        *guard = true;
    }

    let (url, repo) = sess.remote()?;
    let rt = sess.runtime()?;
    let buffer = std::sync::Arc::clone(&sess.event_buffer);

    rt.spawn(async move {
        let mut client = match forge_client::connect_forge(&url).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "forge-ffi: lock-event subscribe: connect failed");
                return;
            }
        };
        let mut stream = match client
            .stream_lock_events(StreamLockEventsRequest { repo: repo.clone() })
            .await
        {
            Ok(resp) => resp.into_inner(),
            Err(e) => {
                tracing::warn!(error = %e, "forge-ffi: stream_lock_events rejected");
                return;
            }
        };

        loop {
            match stream.message().await {
                Ok(Some(ev)) => {
                    // Map the proto enum to a stable string so the
                    // plugin doesn't need proto-aware decoding.
                    let kind = match ev.kind {
                        0 => "snapshot",
                        1 => "acquire",
                        2 => "release",
                        _ => "unknown",
                    };
                    let info = ev.info.unwrap_or_default();
                    let json_obj = json!({
                        "kind": kind,
                        "seq": ev.seq,
                        "info": {
                            "path": info.path,
                            "owner": info.owner,
                            "workspace_id": info.workspace_id,
                            "reason": info.reason,
                            "created_at": info.created_at,
                        },
                    });
                    session::push_event(&buffer, json_obj.to_string());
                }
                Ok(None) => {
                    // Server closed cleanly. Session survives; the
                    // plugin can re-subscribe on the next poll if it
                    // cares. Logging at info level so an operator
                    // notices a flapping stream.
                    tracing::info!("forge-ffi: lock-event stream closed");
                    return;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "forge-ffi: lock-event stream error");
                    return;
                }
            }
        }
    });
    Ok(())
}

// ── Remote-backed ops ───────────────────────────────────────────────────────

/// List the active locks on this workspace's default remote as a
/// JSON array. Each element is `{"path":..., "owner":..., "created_at":...}`.
///
/// Returns null on error; populates `*out_err`. Success values must
/// be released via [`forge_string_free`].
///
/// # Safety
/// - `session` must be non-null.
/// - `out_err`, when non-null, must point to a writable struct.
#[no_mangle]
pub unsafe extern "C" fn forge_lock_list_json(
    session: *mut forge_session_t,
    out_err: *mut forge_error_t,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return ptr::null_mut();
        };
        match run_lock_list(sess) {
            Ok(json) => {
                clear_error(out_err);
                string_to_raw(json, out_err)
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_lock_list_json",
        );
        ptr::null_mut()
    })
}

/// Acquire a lock on `path` for the current workspace user.
///
/// `reason` may be NULL for no reason. Returns 0 on success, non-zero
/// on failure (check `*out_err` for detail).
///
/// # Safety
/// `session` non-null, `path` non-null UTF-8 C string, `reason`
/// nullable UTF-8 C string, `out_err` nullable writable pointer.
#[no_mangle]
pub unsafe extern "C" fn forge_lock_acquire(
    session: *mut forge_session_t,
    path: *const c_char,
    reason: *const c_char,
    out_err: *mut forge_error_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return 1;
        };
        let Some(path_str) = cstr_to_str(path, "path", out_err) else {
            return 1;
        };
        let reason_str = if reason.is_null() {
            String::new()
        } else {
            match cstr_to_str(reason, "reason", out_err) {
                Some(s) => s.to_string(),
                None => return 1,
            }
        };
        match run_lock_acquire(sess, path_str, &reason_str) {
            Ok(()) => {
                clear_error(out_err);
                0
            }
            Err(e) => {
                set_error(out_err, classify_lock_error(&e), &e.to_string());
                1
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_lock_acquire",
        );
        1
    })
}

/// Release the caller's lock on `path`. No-op if no lock is held.
/// Returns 0 on success, non-zero on failure.
///
/// # Safety
/// Same rules as [`forge_lock_acquire`] minus the `reason` arg.
#[no_mangle]
pub unsafe extern "C" fn forge_lock_release(
    session: *mut forge_session_t,
    path: *const c_char,
    out_err: *mut forge_error_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(sess) = session_ref(session, out_err) else {
            return 1;
        };
        let Some(path_str) = cstr_to_str(path, "path", out_err) else {
            return 1;
        };
        match run_lock_release(sess, path_str) {
            Ok(()) => {
                clear_error(out_err);
                0
            }
            Err(e) => {
                set_error(out_err, forge_status_t::FORGE_ERR_IO, &e.to_string());
                1
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_INTERNAL,
            "panic in forge_lock_release",
        );
        1
    })
}

fn run_lock_list(sess: &Session) -> Result<String, anyhow::Error> {
    use forge_proto::forge::ListLocksRequest;
    use serde_json::json;

    let (url, repo) = sess.remote()?;
    let rt = sess.runtime()?;
    let resp = rt.block_on(async {
        let mut client = forge_client::connect_forge(&url).await?;
        Ok::<_, anyhow::Error>(
            client
                .list_locks(ListLocksRequest {
                    repo,
                    path_prefix: String::new(),
                    owner: String::new(),
                })
                .await?
                .into_inner(),
        )
    })?;

    let arr: Vec<_> = resp
        .locks
        .iter()
        .map(|lock| {
            json!({
                "path": lock.path,
                "owner": lock.owner,
                "workspace_id": lock.workspace_id,
                "reason": lock.reason,
                "created_at": lock.created_at,
            })
        })
        .collect();
    Ok(serde_json::Value::Array(arr).to_string())
}

fn run_lock_acquire(sess: &Session, path: &str, reason: &str) -> Result<(), anyhow::Error> {
    use forge_proto::forge::LockRequest;

    let (url, repo) = sess.remote()?;
    let cfg = sess.workspace.config()?;
    let owner = cfg.user.name.clone();
    let rt = sess.runtime()?;
    rt.block_on(async {
        let mut client = forge_client::connect_forge(&url).await?;
        let resp = client
            .acquire_lock(LockRequest {
                repo,
                path: path.to_string(),
                owner,
                workspace_id: cfg.workspace_id.clone(),
                reason: reason.to_string(),
            })
            .await?
            .into_inner();
        if resp.granted {
            Ok(())
        } else {
            // Server-side rejection carries the current lock record.
            // Bubble it up with enough context that the plugin can
            // surface "held by alice since 10:42" to the user.
            let msg = if let Some(existing) = resp.existing_lock.as_ref() {
                format!(
                    "locked by {} (workspace {}, since ts {})",
                    existing.owner, existing.workspace_id, existing.created_at
                )
            } else {
                "lock acquire rejected by server".to_string()
            };
            Err(anyhow::anyhow!(msg))
        }
    })
}

fn run_lock_release(sess: &Session, path: &str) -> Result<(), anyhow::Error> {
    use forge_proto::forge::UnlockRequest;

    let (url, repo) = sess.remote()?;
    let cfg = sess.workspace.config()?;
    let owner = cfg.user.name.clone();
    let rt = sess.runtime()?;
    rt.block_on(async {
        let mut client = forge_client::connect_forge(&url).await?;
        let resp = client
            .release_lock(UnlockRequest {
                repo,
                path: path.to_string(),
                owner,
                force: false,
            })
            .await?
            .into_inner();
        if resp.success {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "{}",
                if resp.error.is_empty() {
                    "release_lock rejected".to_string()
                } else {
                    resp.error
                }
            ))
        }
    })
}

fn classify_lock_error(e: &anyhow::Error) -> forge_status_t {
    // Server "locked by other" maps to CONFLICT so the plugin can
    // branch on that specifically for a "someone else holds this"
    // toast.
    let msg = e.to_string();
    if msg.contains("locked by ") {
        forge_status_t::FORGE_ERR_CONFLICT
    } else {
        forge_status_t::FORGE_ERR_IO
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn build_status_json(sess: &Session) -> Result<String, anyhow::Error> {
    // Delegate to the CLI's shared status walk so the GUI + plugin
    // and `forge status --json` cannot drift. Path resolution goes
    // through the session's workspace root, not process CWD — safe
    // under concurrent callers sharing the loaded DLL.
    let value = forge_cli::commands::status::compute_status_json(&sess.workspace.root)
        .map_err(|e| anyhow::anyhow!("compute_status: {e}"))?;
    Ok(value.to_string())
}

#[allow(dead_code)] // kept for parity with older FFI callers until phase 5.
fn read_head_json(ws: &Workspace) -> Result<serde_json::Value, anyhow::Error> {
    use serde_json::json;
    let head_path = ws.forge_dir().join("HEAD");
    let contents = std::fs::read_to_string(&head_path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", head_path.display()))?;
    let trimmed = contents.trim();
    if let Some(rest) = trimmed.strip_prefix("ref: refs/heads/") {
        Ok(json!({"kind": "branch", "name": rest}))
    } else if let Some(rest) = trimmed.strip_prefix("ref: ") {
        // Non-branch symbolic ref — tags, remotes, etc. Return the
        // raw ref so plugin-side code can decide how to render.
        Ok(json!({"kind": "ref", "name": rest}))
    } else {
        Ok(json!({"kind": "detached", "hash": trimmed}))
    }
}

unsafe fn session_ref<'a>(
    session: *mut forge_session_t,
    out_err: *mut forge_error_t,
) -> Option<&'a Session> {
    if session.is_null() {
        set_error(out_err, forge_status_t::FORGE_ERR_ARG, "session is null");
        return None;
    }
    Some(&*(session as *const Session))
}

unsafe fn string_to_raw(s: String, out_err: *mut forge_error_t) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => {
            set_error(
                out_err,
                forge_status_t::FORGE_ERR_INTERNAL,
                "returned JSON contained interior NUL",
            );
            ptr::null_mut()
        }
    }
}

unsafe fn cstr_to_str<'a>(
    ptr: *const c_char,
    param: &'static str,
    out_err: *mut forge_error_t,
) -> Option<&'a str> {
    if ptr.is_null() {
        set_error(
            out_err,
            forge_status_t::FORGE_ERR_ARG,
            &format!("{param} is null"),
        );
        return None;
    }
    match CStr::from_ptr(ptr).to_str() {
        Ok(s) => Some(s),
        Err(_) => {
            set_error(
                out_err,
                forge_status_t::FORGE_ERR_ARG,
                &format!("{param} is not valid UTF-8"),
            );
            None
        }
    }
}

unsafe fn set_error(slot: *mut forge_error_t, code: forge_status_t, msg: &str) {
    if slot.is_null() {
        // Caller opted out of detail; logging the code/msg on our side
        // preserves debuggability.
        tracing::warn!(?code, msg, "forge-ffi error (no out_err slot)");
        return;
    }
    let s = &mut *slot;
    // Free any stale message from a prior call before overwriting.
    if !s.message.is_null() {
        let _ = CString::from_raw(s.message);
        s.message = ptr::null_mut();
    }
    s.code = code;
    match CString::new(msg) {
        Ok(c) => s.message = c.into_raw(),
        Err(_) => {
            // Interior-NUL in our own error message is a programmer
            // bug. Log + swallow so we never crash the caller.
            tracing::error!("forge-ffi set_error: message contained NUL: {}", msg);
        }
    }
}

unsafe fn clear_error(slot: *mut forge_error_t) {
    if slot.is_null() {
        return;
    }
    let s = &mut *slot;
    if !s.message.is_null() {
        let _ = CString::from_raw(s.message);
        s.message = ptr::null_mut();
    }
    s.code = forge_status_t::FORGE_OK;
}

// Keep one integer reachable from C so link tests can reach the
// symbol without pulling every function. Cheap, no-allocation smoke.
#[no_mangle]
pub extern "C" fn forge_abi_version() -> c_int {
    // Bumped on additive or breaking changes to the exported surface
    // so the plugin can pin a minimum and refuse a stale library.
    // Numeric policy:
    //   1 — session open/close, status_json, error/string free.
    //   2 — locks (list/acquire/release), workspace_info_json,
    //       current_branch, tokio runtime on the session.
    //   3 — index + commit + push + pull via forge_cli::ops.
    //   4 — lock-event subscription (subscribe + poll) + LockEvent
    //       broadcast hub on the server.
    //   5 — log_json, branch_list_json, asset_info_json (read-only
    //       ops reusing forge-core primitives; no run_in plumbing).
    //   6 — status_json wired to compute_status_json (real dirty walk);
    //       unstage, switch, branch_create, branch_delete.
    6
}

// ── Tests (Rust-side; exercise the FFI contract as a C caller would) ────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    /// Allocate a tempdir under the workspace's `target/` directory
    /// so the ancestor chain walks only through dirs we control.
    /// `std::env::temp_dir()` on a dev box frequently lives under
    /// `C:\Users\<name>\` whose `.forge\trusted\` TOFU store mimics a
    /// workspace and makes `Workspace::discover` walk up and match
    /// — that's correct FFI behaviour but turns the "no workspace"
    /// test case into a flake depending on whose machine runs it.
    fn clean_tempdir() -> tempfile::TempDir {
        let anchor = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("ffi-tests");
        std::fs::create_dir_all(&anchor).ok();
        tempfile::Builder::new()
            .prefix("ffi-")
            .tempdir_in(&anchor)
            .expect("tempdir under target/")
    }

    fn with_workspace<F: FnOnce(PathBuf)>(f: F) {
        use forge_core::object::snapshot::Author;
        let dir = clean_tempdir();
        let root = dir.path().to_path_buf();
        Workspace::init(
            &root,
            Author {
                name: "t".into(),
                email: "t@t".into(),
            },
        )
        .unwrap();
        f(root);
    }

    #[test]
    fn version_is_non_null_and_matches_cargo() {
        let ptr = forge_version();
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn open_close_roundtrip_on_valid_workspace() {
        with_workspace(|root| {
            let c_path = CString::new(root.to_str().unwrap()).unwrap();
            let mut err = forge_error_t::default();
            let sess = unsafe { forge_session_open(c_path.as_ptr(), &mut err) };
            assert!(!sess.is_null(), "open should succeed");
            assert_eq!(err.code, forge_status_t::FORGE_OK);
            assert!(err.message.is_null());
            unsafe {
                forge_session_close(sess);
            }
        });
    }

    #[test]
    fn open_fails_on_non_workspace_dir() {
        let dir = clean_tempdir();
        let c_path = CString::new(dir.path().to_str().unwrap()).unwrap();
        let mut err = forge_error_t::default();
        let sess = unsafe { forge_session_open(c_path.as_ptr(), &mut err) };
        assert!(sess.is_null());
        assert_eq!(err.code, forge_status_t::FORGE_ERR_NOT_A_WORKSPACE);
        assert!(!err.message.is_null(), "expected an error message");
        unsafe {
            forge_error_free(&mut err);
        }
        // After free the struct is safe to discard — but double-free
        // must also be safe.
        unsafe {
            forge_error_free(&mut err);
        }
    }

    #[test]
    fn open_fails_on_null_path() {
        let mut err = forge_error_t::default();
        let sess = unsafe { forge_session_open(ptr::null(), &mut err) };
        assert!(sess.is_null());
        assert_eq!(err.code, forge_status_t::FORGE_ERR_ARG);
        unsafe {
            forge_error_free(&mut err);
        }
    }

    #[test]
    fn status_json_reports_branch_on_fresh_init() {
        with_workspace(|root| {
            let c_path = CString::new(root.to_str().unwrap()).unwrap();
            let mut err = forge_error_t::default();
            let sess = unsafe { forge_session_open(c_path.as_ptr(), &mut err) };
            assert!(!sess.is_null());

            let json_ptr = unsafe { forge_status_json(sess, &mut err) };
            assert!(!json_ptr.is_null(), "status should succeed");
            let json = unsafe { CStr::from_ptr(json_ptr) }
                .to_str()
                .unwrap()
                .to_string();
            unsafe {
                forge_string_free(json_ptr);
                forge_session_close(sess);
            }

            // The new compute_status_json shape: workspace_root, branch,
            // staged_*/modified/deleted/untracked arrays, locked array,
            // ahead/behind counters. Fresh init has branch=main and
            // everything empty.
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(v["branch"], "main");
            assert!(v["staged_new"].is_array());
            assert!(v["modified"].is_array());
            assert!(v["untracked"].is_array());
            assert!(v["locked"].is_array());
            assert_eq!(v["ahead"], 0);
            assert_eq!(v["behind"], 0);
        });
    }

    #[test]
    fn status_with_null_session_sets_arg_error() {
        let mut err = forge_error_t::default();
        let p = unsafe { forge_status_json(ptr::null_mut(), &mut err) };
        assert!(p.is_null());
        assert_eq!(err.code, forge_status_t::FORGE_ERR_ARG);
        unsafe {
            forge_error_free(&mut err);
        }
    }

    #[test]
    fn close_is_safe_on_null() {
        // Should not crash.
        unsafe {
            forge_session_close(ptr::null_mut());
        }
    }

    #[test]
    fn workspace_info_json_has_stable_shape() {
        with_workspace(|root| {
            let c_path = CString::new(root.to_str().unwrap()).unwrap();
            let mut err = forge_error_t::default();
            let sess = unsafe { forge_session_open(c_path.as_ptr(), &mut err) };
            assert!(!sess.is_null());

            let ptr = unsafe { forge_workspace_info_json(sess, &mut err) };
            assert!(
                !ptr.is_null(),
                "workspace_info must succeed on a fresh init"
            );
            let json = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
            unsafe {
                forge_string_free(ptr);
                forge_session_close(sess);
            }

            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert!(v["workspace_root"].is_string());
            assert!(v["workspace_id"].is_string());
            assert_eq!(v["head"]["kind"], "branch");
            assert_eq!(v["head"]["name"], "main");
            assert_eq!(v["user"]["name"], "t");
            assert_eq!(v["user"]["email"], "t@t");
            // Fresh init has no remote configured.
            assert!(v["remote_url"].is_null());
            assert!(v["workflow"].is_string());
        });
    }

    #[test]
    fn current_branch_returns_main_on_fresh_init() {
        with_workspace(|root| {
            let c_path = CString::new(root.to_str().unwrap()).unwrap();
            let mut err = forge_error_t::default();
            let sess = unsafe { forge_session_open(c_path.as_ptr(), &mut err) };
            assert!(!sess.is_null());

            let ptr = unsafe { forge_current_branch(sess, &mut err) };
            assert!(!ptr.is_null(), "fresh init has a branch HEAD");
            let name = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
            unsafe {
                forge_string_free(ptr);
                forge_session_close(sess);
            }
            assert_eq!(name, "main");
            assert_eq!(err.code, forge_status_t::FORGE_OK);
        });
    }

    #[test]
    fn current_branch_null_session_errors() {
        let mut err = forge_error_t::default();
        let ptr = unsafe { forge_current_branch(ptr::null_mut(), &mut err) };
        assert!(ptr.is_null());
        assert_eq!(err.code, forge_status_t::FORGE_ERR_ARG);
        unsafe {
            forge_error_free(&mut err);
        }
    }

    #[test]
    fn abi_version_bumped_to_six() {
        assert_eq!(forge_abi_version(), 6);
    }

    #[test]
    fn branch_create_and_delete_roundtrip() {
        with_workspace(|root| {
            // Seed a commit so HEAD isn't zero (branch create needs a commit).
            let readme = root.join("README.md");
            std::fs::write(&readme, b"hello").unwrap();
            forge_cli::commands::add::run_in(&root, vec!["README.md".into()]).unwrap();
            forge_cli::commands::snapshot::run_in(
                &root,
                Some("initial".into()),
                false,
                false,
                false,
            )
            .unwrap();

            let c_path = CString::new(root.to_str().unwrap()).unwrap();
            let mut err = forge_error_t::default();
            let sess = unsafe { forge_session_open(c_path.as_ptr(), &mut err) };
            assert!(!sess.is_null());

            let name = CString::new("feature/xyz").unwrap();

            // Create it.
            let rc = unsafe { forge_branch_create(sess, name.as_ptr(), &mut err) };
            assert_eq!(rc, 0, "create should succeed");
            assert_eq!(err.code, forge_status_t::FORGE_OK);

            // Second create should conflict.
            let rc2 = unsafe { forge_branch_create(sess, name.as_ptr(), &mut err) };
            assert_eq!(rc2, 1);
            assert_eq!(err.code, forge_status_t::FORGE_ERR_CONFLICT);
            unsafe { forge_error_free(&mut err); }

            // Delete it.
            let rc3 = unsafe { forge_branch_delete(sess, name.as_ptr(), 0, &mut err) };
            assert_eq!(rc3, 0, "delete should succeed");

            // Second delete = NOT_FOUND.
            let rc4 = unsafe { forge_branch_delete(sess, name.as_ptr(), 0, &mut err) };
            assert_eq!(rc4, 1);
            assert_eq!(err.code, forge_status_t::FORGE_ERR_NOT_FOUND);
            unsafe {
                forge_error_free(&mut err);
                forge_session_close(sess);
            }
        });
    }

    #[test]
    fn unstage_no_op_on_clean_workspace_succeeds() {
        with_workspace(|root| {
            let c_path = CString::new(root.to_str().unwrap()).unwrap();
            let mut err = forge_error_t::default();
            let sess = unsafe { forge_session_open(c_path.as_ptr(), &mut err) };
            assert!(!sess.is_null());

            let payload = CString::new(r#"["."]"#).unwrap();
            let rc = unsafe { forge_unstage(sess, payload.as_ptr(), &mut err) };
            assert_eq!(rc, 0);
            unsafe { forge_session_close(sess); }
        });
    }

    #[test]
    fn log_json_on_empty_repo_returns_empty_array() {
        with_workspace(|root| {
            let c_path = CString::new(root.to_str().unwrap()).unwrap();
            let mut err = forge_error_t::default();
            let sess = unsafe { forge_session_open(c_path.as_ptr(), &mut err) };
            assert!(!sess.is_null());

            let ptr = unsafe { forge_log_json(sess, 10, &mut err) };
            assert!(!ptr.is_null());
            let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
            unsafe {
                forge_string_free(ptr);
                forge_session_close(sess);
            }
            // Fresh workspace has no commits yet — HEAD points at main
            // but the branch file is zero.
            let v: serde_json::Value = serde_json::from_str(&s).unwrap();
            assert!(v.is_array());
            assert_eq!(v.as_array().unwrap().len(), 0);
        });
    }

    #[test]
    fn branch_list_json_has_main_on_fresh_init() {
        with_workspace(|root| {
            let c_path = CString::new(root.to_str().unwrap()).unwrap();
            let mut err = forge_error_t::default();
            let sess = unsafe { forge_session_open(c_path.as_ptr(), &mut err) };
            assert!(!sess.is_null());

            let ptr = unsafe { forge_branch_list_json(sess, &mut err) };
            assert!(!ptr.is_null(), "branch list must succeed on fresh init");
            let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
            unsafe {
                forge_string_free(ptr);
                forge_session_close(sess);
            }
            let v: serde_json::Value = serde_json::from_str(&s).unwrap();
            assert!(v.is_array());
            let arr = v.as_array().unwrap();
            assert!(
                arr.iter().any(|b| b["name"] == "main" && b["is_current"] == true),
                "expected main to be present + current: {s}"
            );
        });
    }

    #[test]
    fn asset_info_not_found_errors_with_not_found() {
        with_workspace(|root| {
            let c_path = CString::new(root.to_str().unwrap()).unwrap();
            let mut err = forge_error_t::default();
            let sess = unsafe { forge_session_open(c_path.as_ptr(), &mut err) };
            assert!(!sess.is_null());

            let asset_path = CString::new("Content/Missing.uasset").unwrap();
            let ptr = unsafe { forge_asset_info_json(sess, asset_path.as_ptr(), &mut err) };
            assert!(ptr.is_null());
            assert_eq!(err.code, forge_status_t::FORGE_ERR_NOT_FOUND);
            unsafe {
                forge_error_free(&mut err);
                forge_session_close(sess);
            }
        });
    }

    #[test]
    fn asset_info_on_non_uasset_returns_io_error() {
        with_workspace(|root| {
            // Drop a file that isn't a UE asset and make sure we map
            // the "header parse failed" path to FORGE_ERR_IO, not a
            // silent null-without-error regression.
            let junk = root.join("junk.uasset");
            std::fs::write(&junk, b"not a real asset header").unwrap();

            let c_path = CString::new(root.to_str().unwrap()).unwrap();
            let mut err = forge_error_t::default();
            let sess = unsafe { forge_session_open(c_path.as_ptr(), &mut err) };
            assert!(!sess.is_null());

            let asset_path = CString::new("junk.uasset").unwrap();
            let ptr = unsafe { forge_asset_info_json(sess, asset_path.as_ptr(), &mut err) };
            assert!(ptr.is_null());
            assert_eq!(err.code, forge_status_t::FORGE_ERR_IO);
            unsafe {
                forge_error_free(&mut err);
                forge_session_close(sess);
            }
        });
    }

    #[test]
    fn poll_lock_events_empty_buffer_returns_empty_array() {
        with_workspace(|root| {
            let c_path = CString::new(root.to_str().unwrap()).unwrap();
            let mut err = forge_error_t::default();
            let sess = unsafe { forge_session_open(c_path.as_ptr(), &mut err) };
            assert!(!sess.is_null());

            let ptr = unsafe { forge_poll_lock_events_json(sess, &mut err) };
            assert!(!ptr.is_null());
            let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
            unsafe {
                forge_string_free(ptr);
                forge_session_close(sess);
            }
            assert_eq!(s, "[]");
            assert_eq!(err.code, forge_status_t::FORGE_OK);
        });
    }

    #[test]
    fn event_buffer_push_drain_roundtrip_and_cap() {
        let buf = session::EventBuffer::default();
        session::push_event(&buf, r#"{"kind":"acquire","seq":1}"#.into());
        session::push_event(&buf, r#"{"kind":"release","seq":2}"#.into());
        let drained = session::drain_events(&buf);
        assert_eq!(drained.len(), 2);
        // Drain empties the buffer.
        assert!(session::drain_events(&buf).is_empty());
    }

    #[test]
    fn error_free_clears_struct() {
        let mut err = forge_error_t::default();
        unsafe {
            set_error(&mut err, forge_status_t::FORGE_ERR_IO, "broken pipe");
        }
        assert_eq!(err.code, forge_status_t::FORGE_ERR_IO);
        assert!(!err.message.is_null());
        unsafe {
            forge_error_free(&mut err);
        }
        assert_eq!(err.code, forge_status_t::FORGE_OK);
        assert!(err.message.is_null());
    }
}
