// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Shared Forge gRPC client surface.
//!
//! Extracted from `forge-cli` so `forge-ffi` (and any future in-process
//! consumer) can open the same authenticated/trusted gRPC channel
//! without re-implementing credential resolution, pinned-trust TLS,
//! and the Authorization interceptor.
//!
//! Public API:
//! - [`connect_forge`] — the usual `ForgeService` channel.
//! - [`connect_auth`] — the `AuthService` channel (auth interceptor still attached).
//! - [`connect_auth_anonymous`] — bare `AuthService` for login flows where a
//!   stale stored PAT must not leak onto the wire.
//! - [`credentials`] — credential CRUD (keyring → XDG file fallback).
//! - [`url_resolver`] — `https://host/repo` path normalisation + TOFU CA
//!   probe.

pub mod client;
pub mod credentials;
pub mod edge;
pub mod tofu;
pub mod url_resolver;

pub use client::{
    connect_auth, connect_auth_anonymous, connect_forge, connect_forge_write, AuthInterceptor,
};
pub use credentials::Credential;
