// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Integration tests for [`SqliteUserStore`] against a real on-disk SQLite
//! file (the same `MetadataDb::open` path the server uses).

use super::store::*;
use super::tokens::Scope;
use crate::storage::db::MetadataDb;
use std::sync::Arc;
use tempfile::TempDir;

fn fresh_store() -> (TempDir, SqliteUserStore) {
    let tmp = TempDir::new().expect("tempdir");
    let db = MetadataDb::open(&tmp.path().join("forge.db")).expect("open db");
    let store = SqliteUserStore::new(Arc::new(db));
    (tmp, store)
}

fn make_user(store: &SqliteUserStore, username: &str, admin: bool) -> User {
    store
        .create_user(NewUser {
            username: username.into(),
            email: format!("{username}@example.com"),
            display_name: format!("{} Test", username),
            password: "hunter2".into(),
            is_server_admin: admin,
        })
        .expect("create user")
}

// ── Users ────────────────────────────────────────────────────────────────────

#[test]
fn create_and_find_user_by_username() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", true);
    assert_eq!(alice.username, "alice");
    assert!(alice.is_server_admin);

    let found = store.find_user_by_username("alice").unwrap().unwrap();
    assert_eq!(found.id, alice.id);
    assert_eq!(found.email, "alice@example.com");
    assert!(found.is_server_admin);
}

#[test]
fn find_user_by_id() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    let found = store.find_user_by_id(alice.id).unwrap().unwrap();
    assert_eq!(found.username, "alice");
}

#[test]
fn find_missing_user_returns_none() {
    let (_tmp, store) = fresh_store();
    assert!(store.find_user_by_username("ghost").unwrap().is_none());
    assert!(store.find_user_by_id(99999).unwrap().is_none());
}

#[test]
fn duplicate_username_rejected() {
    let (_tmp, store) = fresh_store();
    make_user(&store, "alice", false);
    let dup = store.create_user(NewUser {
        username: "alice".into(),
        email: "other@example.com".into(),
        display_name: "Other".into(),
        password: "hunter2".into(),
        is_server_admin: false,
    });
    assert!(dup.is_err());
}

#[test]
fn list_users_alpha_sorted() {
    let (_tmp, store) = fresh_store();
    make_user(&store, "charlie", false);
    make_user(&store, "alice", true);
    make_user(&store, "bob", false);
    let users = store.list_users().unwrap();
    let names: Vec<&str> = users.iter().map(|u| u.username.as_str()).collect();
    assert_eq!(names, vec!["alice", "bob", "charlie"]);
}

#[test]
fn count_users() {
    let (_tmp, store) = fresh_store();
    assert_eq!(store.count_users().unwrap(), 0);
    make_user(&store, "alice", false);
    make_user(&store, "bob", false);
    assert_eq!(store.count_users().unwrap(), 2);
}

#[test]
fn verify_password_success_and_failure() {
    let (_tmp, store) = fresh_store();
    make_user(&store, "alice", false);
    assert!(store.verify_password("alice", "hunter2").unwrap().is_some());
    assert!(store.verify_password("alice", "wrong").unwrap().is_none());
    assert!(store.verify_password("ghost", "hunter2").unwrap().is_none());
}

#[test]
fn verify_password_updates_last_login_at() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    assert!(alice.last_login_at.is_none());

    store.verify_password("alice", "hunter2").unwrap().unwrap();

    let after = store.find_user_by_id(alice.id).unwrap().unwrap();
    assert!(after.last_login_at.is_some());
}

#[test]
fn set_password_changes_credential() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    store.set_password(alice.id, "newpass").unwrap();
    assert!(store.verify_password("alice", "hunter2").unwrap().is_none());
    assert!(store.verify_password("alice", "newpass").unwrap().is_some());
}

#[test]
fn set_password_rejects_empty() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    assert!(store.set_password(alice.id, "").is_err());
}

#[test]
fn delete_user_returns_true_then_false() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    assert!(store.delete_user(alice.id).unwrap());
    assert!(!store.delete_user(alice.id).unwrap());
    assert!(store.find_user_by_id(alice.id).unwrap().is_none());
}

// ── Sessions ─────────────────────────────────────────────────────────────────

#[test]
fn create_session_and_find_by_plaintext() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    let token = store
        .create_session(alice.id, 3600, Some("test/1.0"), Some("127.0.0.1"))
        .unwrap();

    let (session, user) = store
        .find_session_by_plaintext(&token.plaintext)
        .unwrap()
        .unwrap();
    assert_eq!(session.id, token.session.id);
    assert_eq!(session.user_id, alice.id);
    assert_eq!(user.username, "alice");
}

#[test]
fn find_session_with_wrong_plaintext_returns_none() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    store.create_session(alice.id, 3600, None, None).unwrap();
    // A different token with the same prefix would be vanishingly rare; a
    // totally different prefix is the realistic miss.
    assert!(store
        .find_session_by_plaintext("fses_completely_different")
        .unwrap()
        .is_none());
}

#[test]
fn create_session_rejects_non_positive_ttl() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    assert!(store.create_session(alice.id, 0, None, None).is_err());
    assert!(store.create_session(alice.id, -10, None, None).is_err());
}

#[test]
fn expired_session_does_not_validate() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    let token = store.create_session(alice.id, 3600, None, None).unwrap();
    // Patch expires_at to the past via the test-only db accessor.
    {
        let conn = store.db().conn().unwrap();
        conn.execute(
            "UPDATE sessions SET expires_at = 0 WHERE id = ?1",
            rusqlite::params![token.session.id],
        )
        .unwrap();
    }
    assert!(store
        .find_session_by_plaintext(&token.plaintext)
        .unwrap()
        .is_none());
}

#[test]
fn revoke_session_removes_it() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    let token = store.create_session(alice.id, 3600, None, None).unwrap();
    assert!(store.revoke_session(token.session.id).unwrap());
    assert!(store
        .find_session_by_plaintext(&token.plaintext)
        .unwrap()
        .is_none());
    // Idempotent: revoking again returns false but doesn't error.
    assert!(!store.revoke_session(token.session.id).unwrap());
}

#[test]
fn list_sessions_for_user() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    store.create_session(alice.id, 3600, Some("a"), None).unwrap();
    store.create_session(alice.id, 3600, Some("b"), None).unwrap();
    let sessions = store.list_sessions_for_user(alice.id).unwrap();
    assert_eq!(sessions.len(), 2);
}

// ── PATs ─────────────────────────────────────────────────────────────────────

#[test]
fn create_pat_returns_plaintext_only_once() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    let (pat, plaintext) = store
        .create_pat(alice.id, "ci", &[Scope::RepoRead, Scope::RepoWrite], None)
        .unwrap();
    assert_eq!(pat.name, "ci");
    assert_eq!(pat.scopes, vec![Scope::RepoRead, Scope::RepoWrite]);
    assert!(plaintext.plaintext.starts_with("fpat_"));
    // listing PATs does NOT return the plaintext anywhere
    let listed = store.list_pats_for_user(alice.id).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "ci");
}

#[test]
fn find_pat_by_plaintext_succeeds_for_correct_token() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    let (pat, plaintext) = store
        .create_pat(alice.id, "ci", &[Scope::RepoRead], None)
        .unwrap();
    let (found, user) = store
        .find_pat_by_plaintext(&plaintext.plaintext)
        .unwrap()
        .unwrap();
    assert_eq!(found.id, pat.id);
    assert_eq!(user.username, "alice");
}

#[test]
fn find_pat_with_wrong_plaintext_returns_none() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    store
        .create_pat(alice.id, "ci", &[Scope::RepoRead], None)
        .unwrap();
    assert!(store
        .find_pat_by_plaintext("fpat_definitely_not_real")
        .unwrap()
        .is_none());
}

#[test]
fn create_pat_rejects_empty_scopes() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    assert!(store.create_pat(alice.id, "ci", &[], None).is_err());
}

#[test]
fn create_pat_rejects_empty_name() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    assert!(store
        .create_pat(alice.id, "", &[Scope::RepoRead], None)
        .is_err());
}

#[test]
fn revoke_pat_removes_it() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    let (pat, plaintext) = store
        .create_pat(alice.id, "ci", &[Scope::RepoRead], None)
        .unwrap();
    assert!(store.revoke_pat(pat.id).unwrap());
    assert!(store
        .find_pat_by_plaintext(&plaintext.plaintext)
        .unwrap()
        .is_none());
}

#[test]
fn expired_pat_does_not_validate() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", false);
    let (_, plaintext) = store
        .create_pat(
            alice.id,
            "ci",
            &[Scope::RepoRead],
            Some(chrono::Utc::now().timestamp() - 1),
        )
        .unwrap();
    assert!(store
        .find_pat_by_plaintext(&plaintext.plaintext)
        .unwrap()
        .is_none());
}

// ── Repo ACLs ────────────────────────────────────────────────────────────────

#[test]
fn set_and_get_repo_role() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", true);
    let bob = make_user(&store, "bob", false);
    store
        .set_repo_role("game-data", bob.id, RepoRole::Read, Some(alice.id))
        .unwrap();
    assert_eq!(
        store.get_repo_role("game-data", bob.id).unwrap(),
        Some(RepoRole::Read)
    );
}

#[test]
fn set_repo_role_overwrites_existing() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", true);
    let bob = make_user(&store, "bob", false);
    store
        .set_repo_role("game-data", bob.id, RepoRole::Read, Some(alice.id))
        .unwrap();
    store
        .set_repo_role("game-data", bob.id, RepoRole::Admin, Some(alice.id))
        .unwrap();
    assert_eq!(
        store.get_repo_role("game-data", bob.id).unwrap(),
        Some(RepoRole::Admin)
    );
}

#[test]
fn revoke_repo_role() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", true);
    let bob = make_user(&store, "bob", false);
    store
        .set_repo_role("game-data", bob.id, RepoRole::Write, Some(alice.id))
        .unwrap();
    assert!(store.revoke_repo_role("game-data", bob.id).unwrap());
    assert!(store.get_repo_role("game-data", bob.id).unwrap().is_none());
    assert!(!store.revoke_repo_role("game-data", bob.id).unwrap());
}

#[test]
fn list_repo_members_returns_user_and_role() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", true);
    let bob = make_user(&store, "bob", false);
    let carol = make_user(&store, "carol", false);
    store
        .set_repo_role("game-data", bob.id, RepoRole::Write, Some(alice.id))
        .unwrap();
    store
        .set_repo_role("game-data", carol.id, RepoRole::Admin, Some(alice.id))
        .unwrap();
    let members = store.list_repo_members("game-data").unwrap();
    let by_user: std::collections::HashMap<_, _> = members
        .into_iter()
        .map(|(u, r)| (u.username, r))
        .collect();
    assert_eq!(by_user.get("bob"), Some(&RepoRole::Write));
    assert_eq!(by_user.get("carol"), Some(&RepoRole::Admin));
}

#[test]
fn repo_role_hierarchy() {
    assert!(RepoRole::Admin.satisfies(RepoRole::Read));
    assert!(RepoRole::Admin.satisfies(RepoRole::Write));
    assert!(RepoRole::Admin.satisfies(RepoRole::Admin));
    assert!(RepoRole::Write.satisfies(RepoRole::Read));
    assert!(RepoRole::Write.satisfies(RepoRole::Write));
    assert!(!RepoRole::Write.satisfies(RepoRole::Admin));
    assert!(RepoRole::Read.satisfies(RepoRole::Read));
    assert!(!RepoRole::Read.satisfies(RepoRole::Write));
    assert!(!RepoRole::Read.satisfies(RepoRole::Admin));
}

// ── Cascade behavior ─────────────────────────────────────────────────────────

#[test]
fn deleting_user_cascades_to_sessions_pats_and_acls() {
    let (_tmp, store) = fresh_store();
    let alice = make_user(&store, "alice", true);
    let bob = make_user(&store, "bob", false);
    store.create_session(bob.id, 3600, None, None).unwrap();
    store
        .create_pat(bob.id, "ci", &[Scope::RepoRead], None)
        .unwrap();
    store
        .set_repo_role("game-data", bob.id, RepoRole::Write, Some(alice.id))
        .unwrap();

    // MetadataDb::open enables PRAGMA foreign_keys = ON, so the
    // ON DELETE CASCADE clauses on sessions/pats/repo_acls fire here.

    assert!(store.delete_user(bob.id).unwrap());
    assert!(store.list_sessions_for_user(bob.id).unwrap().is_empty());
    assert!(store.list_pats_for_user(bob.id).unwrap().is_empty());
    assert!(store
        .get_repo_role("game-data", bob.id)
        .unwrap()
        .is_none());
}

// (Test-only helpers live as `#[cfg(test)] pub(crate)` methods on
// `SqliteUserStore` itself in `store.rs` — see `SqliteUserStore::db`.)
