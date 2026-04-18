// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

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

    /// The real struct behind [`super::forge_session_t`]. Only
    /// reachable inside this crate; C sees the opaque tag.
    pub struct Session {
        pub workspace: Workspace,
    }

    impl Session {
        pub fn open(workspace_path: PathBuf) -> Result<Self, anyhow::Error> {
            let workspace = Workspace::discover(&workspace_path)
                .map_err(|e| anyhow::anyhow!("workspace discover: {e}"))?;
            Ok(Self { workspace })
        }
    }
}

use session::Session;

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
                set_error(out_err, forge_status_t::FORGE_ERR_NOT_A_WORKSPACE, &e.to_string());
                ptr::null_mut()
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_error(out_err, forge_status_t::FORGE_ERR_INTERNAL, "panic in forge_session_open");
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

// ── Helpers ─────────────────────────────────────────────────────────────────

fn build_status_json(sess: &Session) -> Result<String, anyhow::Error> {
    use serde_json::json;

    // HEAD: read .forge/HEAD and parse into the branch|detached shape.
    let head = read_head_json(&sess.workspace)?;

    // Phase 4a is the scaffold — the real dirty-file walk lands in
    // 4b once the index + ignore glue is reused through the session.
    // Returning an empty list here keeps the JSON schema stable while
    // the fuller implementation follows.
    Ok(json!({
        "workspace_root": sess.workspace.root.display().to_string(),
        "head": head,
        "dirty": serde_json::Value::Array(Vec::new()),
    })
    .to_string())
}

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
    // Bumped on any ABI-breaking change. Plugin pins a minimum and
    // refuses to load a mismatched library.
    1
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
    fn abi_version_is_one() {
        assert_eq!(forge_abi_version(), 1);
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
    fn status_json_reports_branch_head_on_fresh_init() {
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

            // Fresh init points HEAD at refs/heads/main.
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(v["head"]["kind"], "branch");
            assert_eq!(v["head"]["name"], "main");
            assert!(v["dirty"].is_array());
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
