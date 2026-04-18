// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Per-handler authorization helpers.
//!
//! The interceptor in [`super::interceptor`] only does authentication —
//! it answers "who is this caller?". This module answers "is this caller
//! allowed to do X on resource Y?". Each gRPC handler reads its [`Caller`]
//! and calls one of the `require_*` helpers below to gate the operation.
//!
//! All helpers return [`tonic::Status`] errors so the result can be `?`'d
//! straight up out of a handler.
//!
//! # Roles vs. scopes
//!
//! Two orthogonal checks happen on every authenticated request:
//!
//! 1. **Scope** — does the credential carry the scope this operation
//!    requires? (PATs only — sessions are unscoped and pass implicitly.)
//! 2. **Role** — does the user have the right role on this repo?
//!
//! Both must pass. A `repo:read` PAT can clone every repo the user has read
//! access to, but trying to push will fail the scope check before any role
//! check runs. A `repo:write` PAT held by a user with no role on `secret`
//! will pass the scope check but fail the role check.

use std::sync::Arc;
use tonic::Status;

use super::caller::Caller;
use super::store::{RepoRole, UserStore};
use super::tokens::Scope;

// ── Repo-scoped checks ───────────────────────────────────────────────────────

/// Allow the operation if the caller has at least `read` on `repo`, or if
/// the repo is publicly visible. Used by clone/pull/fetch and all read-only
/// browse endpoints.
///
/// Phase 6 wires up the `visibility` column. Until then, this helper treats
/// every repo as private (the `visibility_lookup` argument is the
/// integration point).
pub fn require_repo_read(
    caller: &Caller,
    store: &Arc<dyn UserStore>,
    repo: &str,
    public: bool,
) -> Result<(), Status> {
    if public {
        return Ok(());
    }
    let auth = require_authenticated(caller)?;
    require_scope(caller, Scope::RepoRead)?;
    if auth.is_server_admin {
        return Ok(());
    }
    let role = store.get_repo_role(repo, auth.user_id).map_err(|e| {
        tracing::error!(error = %e, "repo role lookup");
        Status::internal("internal server error")
    })?;
    match role {
        Some(r) if r.satisfies(RepoRole::Read) => Ok(()),
        _ => Err(Status::permission_denied(format!(
            "no read access to '{repo}'"
        ))),
    }
}

/// Allow the operation if the caller has at least `write` on `repo`. Used by
/// push, ref updates, lock acquire/release, issue/PR mutations, workflow
/// triggers.
pub fn require_repo_write(
    caller: &Caller,
    store: &Arc<dyn UserStore>,
    repo: &str,
) -> Result<(), Status> {
    let auth = require_authenticated(caller)?;
    require_scope(caller, Scope::RepoWrite)?;
    if auth.is_server_admin {
        return Ok(());
    }
    let role = store.get_repo_role(repo, auth.user_id).map_err(|e| {
        tracing::error!(error = %e, "repo role lookup");
        Status::internal("internal server error")
    })?;
    match role {
        Some(r) if r.satisfies(RepoRole::Write) => Ok(()),
        _ => Err(Status::permission_denied(format!(
            "no write access to '{repo}'"
        ))),
    }
}

/// Allow the operation if the caller has `admin` on `repo`. Used by repo
/// rename/delete, ACL management, workflow CRUD, visibility toggles.
pub fn require_repo_admin(
    caller: &Caller,
    store: &Arc<dyn UserStore>,
    repo: &str,
) -> Result<(), Status> {
    let auth = require_authenticated(caller)?;
    require_scope(caller, Scope::RepoAdmin)?;
    if auth.is_server_admin {
        return Ok(());
    }
    let role = store.get_repo_role(repo, auth.user_id).map_err(|e| {
        tracing::error!(error = %e, "repo role lookup");
        Status::internal("internal server error")
    })?;
    match role {
        Some(RepoRole::Admin) => Ok(()),
        _ => Err(Status::permission_denied(format!(
            "no admin access to '{repo}'"
        ))),
    }
}

// ── Server-wide checks ───────────────────────────────────────────────────────

/// Allow the operation only if the caller is a server admin. Used by
/// CreateUser, DeleteUser, ListUsers, CreateRepo (when restricted), and
/// any other server-wide mutation.
pub fn require_server_admin(caller: &Caller) -> Result<(), Status> {
    let auth = require_authenticated(caller)?;
    require_scope(caller, Scope::UserAdmin)?;
    if !auth.is_server_admin {
        return Err(Status::permission_denied("server admin required"));
    }
    Ok(())
}

/// Allow the operation for any logged-in caller. No repo or scope check.
/// Used by `WhoAmI`, `ListMySessions`, `ListPersonalAccessTokens`, and
/// any other "act on my own account" endpoint.
pub fn require_authenticated(
    caller: &Caller,
) -> Result<&super::caller::AuthenticatedCaller, Status> {
    match caller {
        Caller::Authenticated(a) => Ok(a),
        Caller::Anonymous => Err(Status::unauthenticated("login required")),
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn require_scope(caller: &Caller, want: Scope) -> Result<(), Status> {
    if caller.has_scope(want) {
        Ok(())
    } else {
        Err(Status::permission_denied(format!(
            "credential lacks scope '{}'",
            want.as_str()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::caller::{AuthenticatedCaller, CredentialKind};
    use crate::auth::store::SqliteUserStore;
    use crate::auth::{NewUser, Scope};
    use crate::storage::db::MetadataDb;
    use tempfile::TempDir;

    fn fresh() -> (TempDir, Arc<dyn UserStore>) {
        let tmp = TempDir::new().unwrap();
        let db = MetadataDb::open(&tmp.path().join("forge.db")).unwrap();
        let store: Arc<dyn UserStore> = Arc::new(SqliteUserStore::new(Arc::new(db)));
        (tmp, store)
    }

    fn make_user(store: &Arc<dyn UserStore>, username: &str, admin: bool) -> i64 {
        store
            .create_user(NewUser {
                username: username.into(),
                email: format!("{username}@e.com"),
                display_name: username.into(),
                password: "p".into(),
                is_server_admin: admin,
            })
            .unwrap()
            .id
    }

    fn session_caller(uid: i64, username: &str, admin: bool) -> Caller {
        Caller::Authenticated(AuthenticatedCaller {
            user_id: uid,
            username: username.into(),
            is_server_admin: admin,
            scopes: vec![],
            credential: CredentialKind::Session,
        })
    }

    fn pat_caller(uid: i64, username: &str, scopes: Vec<Scope>) -> Caller {
        Caller::Authenticated(AuthenticatedCaller {
            user_id: uid,
            username: username.into(),
            is_server_admin: false,
            scopes,
            credential: CredentialKind::PersonalAccessToken,
        })
    }

    #[test]
    fn anonymous_cannot_read_private_repo() {
        let (_tmp, store) = fresh();
        assert!(require_repo_read(&Caller::anonymous(), &store, "secret", false).is_err());
    }

    #[test]
    fn anonymous_can_read_public_repo() {
        let (_tmp, store) = fresh();
        assert!(require_repo_read(&Caller::anonymous(), &store, "open", true).is_ok());
    }

    #[test]
    fn anonymous_cannot_write_public_repo() {
        let (_tmp, store) = fresh();
        assert!(require_repo_write(&Caller::anonymous(), &store, "open").is_err());
    }

    #[test]
    fn server_admin_session_can_do_anything() {
        let (_tmp, store) = fresh();
        let admin = make_user(&store, "admin", true);
        let c = session_caller(admin, "admin", true);
        assert!(require_repo_read(&c, &store, "any", false).is_ok());
        assert!(require_repo_write(&c, &store, "any").is_ok());
        assert!(require_repo_admin(&c, &store, "any").is_ok());
        assert!(require_server_admin(&c).is_ok());
    }

    #[test]
    fn user_with_no_role_is_denied() {
        let (_tmp, store) = fresh();
        let bob = make_user(&store, "bob", false);
        let c = session_caller(bob, "bob", false);
        assert!(require_repo_read(&c, &store, "secret", false).is_err());
    }

    #[test]
    fn user_with_read_can_read_but_not_write() {
        let (_tmp, store) = fresh();
        let bob = make_user(&store, "bob", false);
        store
            .set_repo_role("game", bob, RepoRole::Read, None)
            .unwrap();
        let c = session_caller(bob, "bob", false);
        assert!(require_repo_read(&c, &store, "game", false).is_ok());
        assert!(require_repo_write(&c, &store, "game").is_err());
        assert!(require_repo_admin(&c, &store, "game").is_err());
    }

    #[test]
    fn user_with_write_can_read_and_write_but_not_admin() {
        let (_tmp, store) = fresh();
        let bob = make_user(&store, "bob", false);
        store
            .set_repo_role("game", bob, RepoRole::Write, None)
            .unwrap();
        let c = session_caller(bob, "bob", false);
        assert!(require_repo_read(&c, &store, "game", false).is_ok());
        assert!(require_repo_write(&c, &store, "game").is_ok());
        assert!(require_repo_admin(&c, &store, "game").is_err());
    }

    #[test]
    fn user_with_admin_passes_all_repo_checks() {
        let (_tmp, store) = fresh();
        let bob = make_user(&store, "bob", false);
        store
            .set_repo_role("game", bob, RepoRole::Admin, None)
            .unwrap();
        let c = session_caller(bob, "bob", false);
        assert!(require_repo_read(&c, &store, "game", false).is_ok());
        assert!(require_repo_write(&c, &store, "game").is_ok());
        assert!(require_repo_admin(&c, &store, "game").is_ok());
        // But not server admin
        assert!(require_server_admin(&c).is_err());
    }

    #[test]
    fn pat_scope_gates_operation() {
        let (_tmp, store) = fresh();
        let bob = make_user(&store, "bob", false);
        store
            .set_repo_role("game", bob, RepoRole::Write, None)
            .unwrap();
        // PAT with only repo:read — push must fail at scope check
        let c = pat_caller(bob, "bob", vec![Scope::RepoRead]);
        assert!(require_repo_read(&c, &store, "game", false).is_ok());
        assert!(require_repo_write(&c, &store, "game").is_err());
    }

    #[test]
    fn pat_with_write_scope_can_write() {
        let (_tmp, store) = fresh();
        let bob = make_user(&store, "bob", false);
        store
            .set_repo_role("game", bob, RepoRole::Write, None)
            .unwrap();
        let c = pat_caller(bob, "bob", vec![Scope::RepoRead, Scope::RepoWrite]);
        assert!(require_repo_write(&c, &store, "game").is_ok());
    }

    #[test]
    fn non_admin_pat_cannot_pass_server_admin_check() {
        let (_tmp, store) = fresh();
        let bob = make_user(&store, "bob", false);
        let c = pat_caller(bob, "bob", vec![Scope::UserAdmin]);
        // Has the scope but is_server_admin is false
        assert!(require_server_admin(&c).is_err());
    }

    #[test]
    fn admin_user_with_pat_lacking_user_admin_scope_fails() {
        let (_tmp, store) = fresh();
        let admin = make_user(&store, "admin", true);
        // is_server_admin = true on the user, but the PAT only carries
        // repo:read — server admin check still requires the user:admin scope
        // because scopes exist to cap a leaked PAT's blast radius.
        let c = Caller::Authenticated(AuthenticatedCaller {
            user_id: admin,
            username: "admin".into(),
            is_server_admin: true,
            scopes: vec![Scope::RepoRead],
            credential: CredentialKind::PersonalAccessToken,
        });
        assert!(require_server_admin(&c).is_err());
    }
}
