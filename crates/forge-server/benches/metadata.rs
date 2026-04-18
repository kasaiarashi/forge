// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under BSL 1.1.

//! Phase 7f — single-threaded hot-path benches for the metadata
//! surface. These are not regression gates on their own; they're a
//! baseline so future changes (new pragmas, indexes, Postgres swap,
//! etc) have a reference number.
//!
//! Run with `cargo bench --release -p forge-server --bench metadata`.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use tempfile::TempDir;

use forge_server::storage::db::{MetadataDb, RefUpdateSpec};

const ZERO: [u8; 32] = [0u8; 32];

fn h(byte: u8) -> Vec<u8> {
    vec![byte; 32]
}

/// Every bench starts from an empty DB under a fresh temp dir. Keeping
/// the TempDir alive for the duration of the bench is critical —
/// dropping it deletes the SQLite file, and Windows' deferred-delete
/// semantics can wedge the next iteration mid-run.
fn fresh() -> (TempDir, MetadataDb) {
    let tmp = TempDir::new().expect("tempdir");
    let db = MetadataDb::open(&tmp.path().join("forge.db")).expect("open db");
    db.create_repo("bench/repo", "bench").unwrap();
    (tmp, db)
}

fn bench_ref_update_cas(c: &mut Criterion) {
    c.bench_function("ref_update_cas_single_ref", |b| {
        let (_tmp, db) = fresh();
        // Seed ref so every iteration is a pure CAS write.
        db.update_ref("bench/repo", "refs/heads/main", &ZERO, &h(0x01), false)
            .unwrap();
        let mut state: u8 = 0x01;
        b.iter(|| {
            let next = state.wrapping_add(1);
            let ok = db
                .update_ref(
                    "bench/repo",
                    "refs/heads/main",
                    &h(state),
                    &h(next),
                    false,
                )
                .unwrap();
            assert!(ok);
            state = next;
        });
    });

    c.bench_function("ref_update_cas_stale_fail", |b| {
        let (_tmp, db) = fresh();
        db.update_ref("bench/repo", "refs/heads/main", &ZERO, &h(0xAA), false)
            .unwrap();
        b.iter(|| {
            // CAS with the wrong old_hash — must return false without
            // touching the ref. Measures the "cheap" branch of the
            // CAS SQL.
            let ok = db
                .update_ref(
                    "bench/repo",
                    "refs/heads/main",
                    &h(0x01),
                    &h(0x02),
                    false,
                )
                .unwrap();
            assert!(!ok);
        });
    });
}

fn bench_lock_acquire_release(c: &mut Criterion) {
    c.bench_function("lock_acquire_release", |b| {
        let (_tmp, db) = fresh();
        b.iter_batched(
            || (),
            |_| {
                db.acquire_lock(
                    "bench/repo",
                    "Content/Maps/main.umap",
                    "alice",
                    "ws-1",
                    "",
                )
                .unwrap()
                .unwrap();
                let ok = db
                    .release_lock("bench/repo", "Content/Maps/main.umap", "alice", false)
                    .unwrap();
                assert!(ok);
            },
            BatchSize::SmallInput,
        );
    });

    c.bench_function("lock_acquire_already_held", |b| {
        let (_tmp, db) = fresh();
        db.acquire_lock(
            "bench/repo",
            "Content/Maps/main.umap",
            "alice",
            "ws-1",
            "",
        )
        .unwrap()
        .unwrap();
        b.iter(|| {
            // Same owner + path — the fast path returns Ok(Ok(()))
            // after the SELECT without hitting the INSERT branch.
            db.acquire_lock(
                "bench/repo",
                "Content/Maps/main.umap",
                "alice",
                "ws-1",
                "",
            )
            .unwrap()
            .unwrap();
        });
    });

    c.bench_function("list_locks_1k", |b| {
        let (_tmp, db) = fresh();
        for i in 0..1000 {
            db.acquire_lock(
                "bench/repo",
                &format!("Content/lock{i}.umap"),
                "alice",
                "ws-1",
                "",
            )
            .unwrap()
            .unwrap();
        }
        b.iter(|| {
            let locks = db.list_locks("bench/repo", "", "").unwrap();
            black_box(locks);
        });
    });
}

fn bench_metrics_snapshot(c: &mut Criterion) {
    c.bench_function("metrics_snapshot_cold", |b| {
        let (_tmp, db) = fresh();
        b.iter(|| {
            let snap = db.metrics_snapshot().unwrap();
            black_box(snap);
        });
    });

    c.bench_function("metrics_snapshot_with_load", |b| {
        let (_tmp, db) = fresh();
        // Prime the tables so the COUNT(*)s exercise real pages.
        for i in 0..1000 {
            db.acquire_lock(
                "bench/repo",
                &format!("Content/lock{i}.umap"),
                "alice",
                "ws-1",
                "",
            )
            .unwrap()
            .unwrap();
        }
        for i in 0..200 {
            db.create_upload_session(&format!("sess-{i}"), "bench/repo", None, 3600)
                .unwrap();
        }
        b.iter(|| {
            let snap = db.metrics_snapshot().unwrap();
            black_box(snap);
        });
    });
}

fn bench_upload_session_commit(c: &mut Criterion) {
    c.bench_function("upload_session_full_commit", |b| {
        b.iter_batched(
            fresh,
            |(_tmp, db)| {
                let sid = "sess-bench";
                db.create_upload_session(sid, "bench/repo", None, 3600).unwrap();
                db.record_session_object(sid, &h(0xAA), 128).unwrap();
                db.record_session_object(sid, &h(0xBB), 256).unwrap();
                let new_hash = h(0xAA);
                let updates = vec![RefUpdateSpec {
                    ref_name: "refs/heads/main",
                    old_hash: &ZERO,
                    new_hash: &new_hash,
                    force: false,
                }];
                let outcome = db.commit_upload_session(sid, &updates).unwrap();
                black_box(outcome);
            },
            BatchSize::LargeInput,
        );
    });
}

criterion_group!(
    benches,
    bench_ref_update_cas,
    bench_lock_acquire_release,
    bench_metrics_snapshot,
    bench_upload_session_commit,
);
criterion_main!(benches);
