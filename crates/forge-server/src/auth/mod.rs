// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

// Phase 1 ships the persistence layer that phases 2/3/4 will consume.
// Until those land, most items in this module have no in-crate caller.
// Lift this attribute once the gRPC interceptor and CLI subcommands wire
// the auth module up.
#![allow(dead_code)]

//! Authentication and authorization for forge-server.
//!
//! This module owns all user/session/PAT/ACL state. The CLI and the web UI
//! both consume it indirectly through the gRPC `AuthService` (wired in phase 3).
//!
//! Phase 1 ships the persistence layer only:
//!
//! - [`password`] — argon2id password and token hashing.
//! - [`tokens`]   — random token generation, prefix splitting, scope parsing.
//! - [`caller`]   — the [`Caller`](caller::Caller) value attached to every
//!                  authenticated gRPC request.
//! - [`store`]    — the [`UserStore`](store::UserStore) trait + the
//!                  [`SqliteUserStore`](store::SqliteUserStore) implementation.
//!
//! Phases 2/3/4 add the CLI subcommands, the gRPC interceptor, the per-handler
//! authorization helpers, and the client wiring.

pub mod authorize;
pub mod caller;
pub mod interceptor;
pub mod password;
pub mod store;
#[cfg(feature = "postgres")]
pub mod store_postgres;
pub mod tokens;

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "postgres-tests"))]
mod tests_postgres;

// Phase-1 ergonomic re-exports. Downstream phases (interceptor, gRPC service,
// CLI subcommands) will import from here rather than the submodules. Marked
// allow(unused_imports) so the build stays clean while those phases are still
// in flight — once they land, the warnings disappear naturally.
#[allow(unused_imports)]
pub use caller::{AuthenticatedCaller, Caller, CredentialKind};
#[allow(unused_imports)]
pub use store::{
    NewUser, PersonalAccessToken, RepoRole, Session, SessionToken, SqliteUserStore, User, UserStore,
};
#[allow(unused_imports)]
pub use tokens::{PatPlaintext, Scope};
