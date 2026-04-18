// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under BSL 1.1.

//! PgUserStore parity tests.
//!
//! Mirrors enough of `auth::tests` to prove the Postgres impl behaves
//! identically to SqliteUserStore for the scenarios the server's
//! interceptor + admin handlers actually exercise. Skipped when
//! `DATABASE_URL` is unset so local `cargo test` works without a
//! Postgres dependency.
//!
//! Run with:
//! ```text
//! DATABASE_URL='postgres://forge:forge@127.0.0.1:5433/forge' \
//!   cargo test --release -p forge-server --features postgres-tests \
//!   --lib auth::tests_postgres -- --test-threads=1
//! ```
//! `--test-threads=1` because each test wipes the schema; running them
//! in parallel races on table creation.

#![cfg(feature = "postgres-tests")]

use std::sync::Arc;

use super::store::{NewUser, RepoRole, UserStore};
use super::store_postgres::PgUserStore;
use super::tokens::Scope;
use crate::storage::postgres::{PgMetadataBackend, PgPoolConfig};

fn database_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty())
}

fn reset(url: &str) {
    let mut client = ::postgres::Client::connect(url, ::postgres::NoTls)
        .expect("connect to DATABASE_URL for reset");
    client
        .batch_execute(
            "DROP TABLE IF EXISTS pending_repo_ops CASCADE;
             DROP TABLE IF EXISTS session_objects CASCADE;
             DROP TABLE IF EXISTS upload_sessions CASCADE;
             DROP TABLE IF EXISTS repo_acls CASCADE;
             DROP TABLE IF EXISTS personal_access_tokens CASCADE;
             DROP TABLE IF EXISTS sessions CASCADE;
             DROP TABLE IF EXISTS users CASCADE;
             DROP TABLE IF EXISTS locks CASCADE;
             DROP TABLE IF EXISTS refs CASCADE;
             DROP TABLE IF EXISTS repos CASCADE;
             DROP TABLE IF EXISTS schema_version CASCADE;",
        )
        .expect("reset baseline tables");
}

fn fresh_store() -> Option<PgUserStore> {
    let url = database_url()?;
    reset(&url);
    let pg = PgMetadataBackend::open(PgPoolConfig {
        url,
        ..Default::default()
    })
    .expect("open postgres backend");
    Some(PgUserStore::new(Arc::new(pg)))
}

fn make_user(store: &PgUserStore, username: &str, admin: bool) -> super::store::User {
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

#[test]
fn create_and_lookup_user() {
    let Some(store) = fresh_store() else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let alice = make_user(&store, "alice", false);
    let bob = make_user(&store, "bob", true);

    let by_name = store.find_user_by_username("alice").unwrap().unwrap();
    assert_eq!(by_name.id, alice.id);
    assert!(!by_name.is_server_admin);

    let by_id = store.find_user_by_id(bob.id).unwrap().unwrap();
    assert!(by_id.is_server_admin);

    let users = store.list_users().unwrap();
    assert_eq!(users.len(), 2);
    assert_eq!(store.count_users().unwrap(), 2);
}

#[test]
fn verify_password_round_trip() {
    let Some(store) = fresh_store() else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let _alice = make_user(&store, "alice", false);

    assert!(store.verify_password("alice", "wrong").unwrap().is_none());
    let ok = store.verify_password("alice", "hunter2").unwrap().unwrap();
    assert_eq!(ok.username, "alice");
    // last_login_at should be populated after verify.
    let after = store.find_user_by_username("alice").unwrap().unwrap();
    assert!(after.last_login_at.is_some());
}

#[test]
fn create_session_and_find_by_plaintext() {
    let Some(store) = fresh_store() else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let alice = make_user(&store, "alice", false);
    let token = store
        .create_session(alice.id, 3600, Some("ua"), Some("127.0.0.1"))
        .unwrap();
    let found = store
        .find_session_by_plaintext(&token.plaintext)
        .unwrap()
        .unwrap();
    assert_eq!(found.0.id, token.session.id);
    assert_eq!(found.1.id, alice.id);
}

#[test]
fn pat_create_then_find_then_revoke() {
    let Some(store) = fresh_store() else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let alice = make_user(&store, "alice", false);
    let scopes = vec![Scope::RepoRead];
    let (pat, plaintext) = store
        .create_pat(alice.id, "ci", &scopes, None)
        .unwrap();
    let found = store
        .find_pat_by_plaintext(&plaintext.plaintext)
        .unwrap()
        .unwrap();
    assert_eq!(found.0.id, pat.id);
    assert_eq!(found.1.id, alice.id);

    assert!(store.revoke_pat(pat.id).unwrap());
    assert!(store
        .find_pat_by_plaintext(&plaintext.plaintext)
        .unwrap()
        .is_none());
}

#[test]
fn repo_role_grant_and_revoke() {
    let Some(store) = fresh_store() else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let alice = make_user(&store, "alice", false);
    store
        .set_repo_role("alice/forge", alice.id, RepoRole::Write, None)
        .unwrap();
    let role = store
        .get_repo_role("alice/forge", alice.id)
        .unwrap()
        .unwrap();
    assert_eq!(role, RepoRole::Write);

    // Update.
    store
        .set_repo_role("alice/forge", alice.id, RepoRole::Admin, None)
        .unwrap();
    let role = store
        .get_repo_role("alice/forge", alice.id)
        .unwrap()
        .unwrap();
    assert_eq!(role, RepoRole::Admin);

    let members = store.list_repo_members("alice/forge").unwrap();
    assert_eq!(members.len(), 1);

    assert!(store.revoke_repo_role("alice/forge", alice.id).unwrap());
    assert!(store
        .get_repo_role("alice/forge", alice.id)
        .unwrap()
        .is_none());
}
