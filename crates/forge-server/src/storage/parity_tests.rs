// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Cross-backend parity suite for the Phase-1 atomic-push surface.
//!
//! The same generic exerciser runs against both backends. SQLite runs
//! unconditionally; Postgres runs only when `DATABASE_URL` is set AND
//! the `postgres-tests` Cargo feature is on so local `cargo test`
//! works without a Postgres dependency.
//!
//! Contract: if either backend diverges from the expected trait
//! semantics, this suite fails loudly. That's the whole reason the
//! trait exists.

use crate::storage::backend::MetadataBackend;
use crate::storage::db::{CommitSessionOutcome, RefUpdateSpec};

fn zeros32() -> Vec<u8> {
    vec![0u8; 32]
}
fn hash(byte: u8) -> Vec<u8> {
    vec![byte; 32]
}

/// Exercises the full Phase-1 atomic-push surface against any
/// [`MetadataBackend`]. Ordered so each step builds on the previous.
fn exercise_atomic_push_surface<B: MetadataBackend>(backend: &B) {
    let repo = "alice/parity";

    // -- Repos --
    assert!(backend.create_repo(repo, "parity suite").unwrap());
    assert!(
        !backend.create_repo(repo, "dup").unwrap(),
        "duplicate create must be idempotent (return false, not error)",
    );
    let repos = backend.list_repos().unwrap();
    assert!(repos.iter().any(|r| r.name == repo));
    assert_eq!(
        backend.get_repo_visibility(repo).unwrap().as_deref(),
        Some("private"),
    );
    assert!(!backend.is_repo_public(repo));
    assert!(backend.set_repo_visibility(repo, "public").unwrap());
    assert!(backend.is_repo_public(repo));

    // -- Upload session + commit path --
    let sid = "sess-parity-1";
    backend
        .create_upload_session(sid, repo, None, 3600)
        .unwrap();
    // Idempotent re-create.
    backend
        .create_upload_session(sid, repo, None, 3600)
        .unwrap();

    let h1 = hash(0xAA);
    let h2 = hash(0xBB);
    backend.record_session_object(sid, &h1, 128).unwrap();
    backend.record_session_object(sid, &h2, 256).unwrap();
    // Duplicate record = no-op.
    backend.record_session_object(sid, &h1, 128).unwrap();

    let recorded = backend.list_session_object_hashes(sid).unwrap();
    assert_eq!(recorded.len(), 2);
    assert!(recorded.contains(&h1));
    assert!(recorded.contains(&h2));

    // list_session_objects_with_sizes backs the QueryUploadSession
    // resume path — it must return hash + declared size so the
    // client knows how much to resend.
    let with_sizes = backend.list_session_objects_with_sizes(sid).unwrap();
    assert_eq!(with_sizes.len(), 2);
    let size_map: std::collections::HashMap<Vec<u8>, i64> = with_sizes.into_iter().collect();
    assert_eq!(size_map.get(&h1), Some(&128));
    assert_eq!(size_map.get(&h2), Some(&256));

    let before_commit = backend.get_upload_session(sid).unwrap().unwrap();
    assert_eq!(before_commit.state, "uploading");

    // Commit: create a new ref `refs/heads/main` pointing at h1.
    let zeros = zeros32();
    let main_ref = "refs/heads/main";
    let updates = vec![RefUpdateSpec {
        ref_name: main_ref,
        old_hash: &zeros,
        new_hash: &h1,
        force: false,
    }];
    let outcome = backend.commit_upload_session(sid, &updates).unwrap();
    match outcome {
        CommitSessionOutcome::Committed {
            all_success,
            ref_results,
        } => {
            assert!(all_success, "ref create should succeed: {ref_results:?}");
        }
        other => panic!("expected Committed, got {other:?}"),
    }

    let after_commit = backend.get_upload_session(sid).unwrap().unwrap();
    assert_eq!(after_commit.state, "committed");
    assert!(after_commit.committed_at.is_some());

    // Idempotent replay.
    let replay = backend.commit_upload_session(sid, &updates).unwrap();
    assert!(
        matches!(replay, CommitSessionOutcome::AlreadyCommitted { .. }),
        "retry of committed session must replay cached result",
    );

    // -- Refs --
    let main_after = backend.get_ref(repo, main_ref).unwrap().unwrap();
    assert_eq!(main_after, h1);

    let all_refs = backend.get_all_refs(repo).unwrap();
    assert_eq!(all_refs.len(), 1);
    assert_eq!(all_refs[0].0, main_ref);

    // CAS: old matches, so success.
    assert!(
        backend.update_ref(repo, main_ref, &h1, &h2, false).unwrap(),
        "CAS with correct old_hash must succeed",
    );
    assert_eq!(backend.get_ref(repo, main_ref).unwrap().unwrap(), h2);

    // CAS: old mismatches, so failure.
    assert!(
        !backend
            .update_ref(repo, main_ref, &h1, &hash(0xCC), false)
            .unwrap(),
        "CAS with stale old_hash must fail",
    );
    assert_eq!(backend.get_ref(repo, main_ref).unwrap().unwrap(), h2);

    // Force update bypasses CAS.
    assert!(
        backend
            .update_ref(repo, main_ref, &h1, &hash(0xCC), true)
            .unwrap(),
        "force update must ignore old_hash",
    );
    assert_eq!(
        backend.get_ref(repo, main_ref).unwrap().unwrap(),
        hash(0xCC)
    );

    // -- Locks --
    let lock_path = "Content/Hero.uasset";
    let first = backend
        .acquire_lock(repo, lock_path, "alice", "ws-1", "editing")
        .unwrap();
    assert!(first.is_ok(), "first acquire should succeed");

    // Same owner re-acquire = ok.
    let dup = backend
        .acquire_lock(repo, lock_path, "alice", "ws-1", "editing")
        .unwrap();
    assert!(dup.is_ok(), "re-acquire by same owner must be idempotent");

    // Different owner = denied with lock info.
    let denied = backend
        .acquire_lock(repo, lock_path, "bob", "ws-2", "retexture")
        .unwrap();
    match denied {
        Err(existing) => assert_eq!(existing.owner, "alice"),
        Ok(()) => panic!("bob should not acquire a lock alice holds"),
    }

    let locks = backend.list_locks(repo, "Content/", "").unwrap();
    assert_eq!(locks.len(), 1);
    assert_eq!(locks[0].owner, "alice");

    // Non-owner release without force = no-op.
    assert!(
        !backend.release_lock(repo, lock_path, "bob", false).unwrap(),
        "non-owner release must fail (returns false)",
    );
    // Owner release = ok.
    assert!(backend
        .release_lock(repo, lock_path, "alice", false)
        .unwrap());
    assert!(backend.list_locks(repo, "", "").unwrap().is_empty());

    // -- Stale session sweep --
    let far_future = chrono::Utc::now().timestamp() + 3600 * 24;
    let stale = backend.list_stale_upload_sessions(far_future).unwrap();
    assert!(
        stale.iter().any(|(s, r)| s == sid && r == repo),
        "committed session should appear in stale sweep when cutoff is past it",
    );

    backend.delete_upload_session(sid).unwrap();
    assert!(backend.get_upload_session(sid).unwrap().is_none());

    // -- Pending repo ops (Phase 3b.5 drain queue) --
    //
    // Verifies the claim / complete / fail / backoff contract that
    // `services::repo_ops_drain` relies on. Both backends must
    // deliver identical semantics here — rename/delete queued on a
    // SQLite deployment must drain the same way it would on Postgres.

    let initial = backend.list_pending_repo_ops().unwrap();
    let initial_len = initial.len();

    let rename_id = backend
        .enqueue_repo_op("rename", "alice/old", Some("alice/new"))
        .unwrap();
    let delete_id = backend
        .enqueue_repo_op("delete", "alice/doomed", None)
        .unwrap();
    assert_ne!(rename_id, delete_id);

    let listed = backend.list_pending_repo_ops().unwrap();
    assert_eq!(listed.len(), initial_len + 2);

    // Claim: oldest-first, so we get the rename first.
    let first = backend
        .claim_next_repo_op(60)
        .unwrap()
        .expect("queue has pending ops");
    assert_eq!(first.id, rename_id);
    assert_eq!(first.op_type, "rename");
    assert_eq!(first.repo, "alice/old");
    assert_eq!(first.new_repo.as_deref(), Some("alice/new"));
    assert_eq!(first.attempts, 1, "claim must bump attempts to 1");

    // A second claim skips the one we just claimed (hidden by
    // visibility timeout) and returns the delete op instead.
    let second = backend
        .claim_next_repo_op(60)
        .unwrap()
        .expect("delete op still queued");
    assert_eq!(second.id, delete_id);
    assert_eq!(second.op_type, "delete");
    assert!(second.new_repo.is_none());

    // With both claimed + hidden, the queue looks empty.
    assert!(
        backend.claim_next_repo_op(60).unwrap().is_none(),
        "all ops are inside their visibility window",
    );

    // Fail the rename with a 0-second backoff so it becomes
    // immediately re-claimable. `attempts` keeps climbing.
    backend
        .fail_repo_op(rename_id, "simulated failure", 0)
        .unwrap();
    let retried = backend
        .claim_next_repo_op(60)
        .unwrap()
        .expect("failed op becomes eligible after its retry delay");
    assert_eq!(retried.id, rename_id);
    assert_eq!(retried.attempts, 2, "retry must bump attempts again");

    // Complete both. list_pending_repo_ops drops to the pre-test
    // baseline.
    backend.complete_repo_op(rename_id).unwrap();
    backend.complete_repo_op(delete_id).unwrap();
    assert_eq!(
        backend.list_pending_repo_ops().unwrap().len(),
        initial_len,
        "complete_repo_op must delete the row",
    );

    // Bad op_type is rejected at enqueue time.
    assert!(
        backend
            .enqueue_repo_op("vaporize", "alice/old", None)
            .is_err(),
        "enqueue must validate op_type",
    );

    // -- Cleanup (schema_version survives; repo + children gone) --
    assert!(backend.delete_repo(repo).unwrap());
    assert!(backend.list_repos().unwrap().iter().all(|r| r.name != repo));

    // -- Migration runner smoke --
    let v = backend.current_schema_version().unwrap();
    assert!(v >= 1, "migrations must have run, got {v}");
}

// ── SQLite ──────────────────────────────────────────────────────────────

#[test]
fn sqlite_atomic_push_parity() {
    use crate::storage::db::MetadataDb;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let db = MetadataDb::open(&tmp.path().join("forge.db")).unwrap();
    exercise_atomic_push_surface(&db);
}

// ── Postgres ────────────────────────────────────────────────────────────

#[cfg(feature = "postgres-tests")]
mod pg {
    use super::*;
    use crate::storage::postgres::{PgMetadataBackend, PgPoolConfig};

    /// Wipe every Phase-1 baseline table so the test run starts
    /// clean. CI must point `DATABASE_URL` at a dedicated throwaway
    /// database — running this against a shared prod-like instance
    /// is a data-loss event waiting to happen.
    fn reset(url: &str) {
        let mut client = ::postgres::Client::connect(url, ::postgres::NoTls)
            .expect("connect to DATABASE_URL for reset");
        client
            .batch_execute(
                "DROP TABLE IF EXISTS pending_repo_ops CASCADE;
                 DROP TABLE IF EXISTS session_objects CASCADE;
                 DROP TABLE IF EXISTS upload_sessions CASCADE;
                 DROP TABLE IF EXISTS locks CASCADE;
                 DROP TABLE IF EXISTS refs CASCADE;
                 DROP TABLE IF EXISTS repos CASCADE;
                 DROP TABLE IF EXISTS schema_version CASCADE;",
            )
            .expect("reset baseline tables");
    }

    fn database_url() -> Option<String> {
        std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty())
    }

    #[test]
    fn postgres_atomic_push_parity() {
        let Some(url) = database_url() else {
            eprintln!("SKIP: DATABASE_URL not set");
            return;
        };
        reset(&url);

        let cfg = PgPoolConfig {
            url: url.clone(),
            ..Default::default()
        };
        let backend = PgMetadataBackend::open(cfg).expect("open postgres backend");
        exercise_atomic_push_surface(&backend);
    }

    #[test]
    fn postgres_migration_is_idempotent() {
        let Some(url) = database_url() else {
            eprintln!("SKIP: DATABASE_URL not set");
            return;
        };
        reset(&url);

        let cfg = PgPoolConfig {
            url,
            ..Default::default()
        };
        let b1 = PgMetadataBackend::open(cfg.clone()).unwrap();
        let v1 = b1.current_schema_version().unwrap();
        drop(b1);

        let b2 = PgMetadataBackend::open(cfg).unwrap();
        let v2 = b2.current_schema_version().unwrap();
        assert_eq!(v1, v2, "re-open must not re-advance schema_version");
    }
}
