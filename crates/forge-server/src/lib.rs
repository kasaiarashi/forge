// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under BSL 1.1.

//! Library face of forge-server.
//!
//! Exists so external benches + load-test harnesses can hit the same
//! `MetadataDb` / `FsStorage` modules the binary runs without
//! forking the implementation. The `forge-server` binary still lives
//! in `main.rs` and links the same source via its own `mod` tree —
//! the duplicated compile is the cost of letting
//! `crate::storage::db::MetadataDb` resolve identically in either
//! crate root.
//!
//! Surface is intentionally narrow: only the modules that benches /
//! harnesses use are re-exported here. Bin-only modules (Windows
//! service plumbing, TLS autogen, updater, etc) stay in the binary
//! crate alone — re-exporting them would force them to compile
//! against `crate::serve_inner` which only exists in `main.rs`.

pub mod auth;
pub mod config;
// observability is here for the `#[macro_export] audit!` macro that
// services/* call as `crate::audit!(...)`. Without it the lib build
// fails with `no audit in the root`. The non-macro init() helper is
// bin-only by convention.
pub mod observability;
pub mod services;
pub mod storage;
