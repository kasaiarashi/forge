// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under BSL 1.1.

//! Phase 7f — multi-threaded contention benches.
//!
//! These exist to catch regressions in the r2d2 pool + `BEGIN
//! IMMEDIATE` story that Phase 2a established. A single-threaded
//! micro-bench misses the pool starvation / busy-timeout failure
//! modes that show up under N=32 push pressure.
//!
//! Run with `cargo bench --release -p forge-server --bench concurrency`.

use std::sync::Arc;
use std::thread;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use tempfile::TempDir;

use forge_server::storage::db::MetadataDb;

fn fresh_db() -> (TempDir, Arc<MetadataDb>) {
    let tmp = TempDir::new().expect("tempdir");
    let db = Arc::new(MetadataDb::open(&tmp.path().join("forge.db")).expect("open db"));
    db.create_repo("bench/repo", "").unwrap();
    (tmp, db)
}

/// N threads each acquire+release a unique lock path. Measures the
/// pooled-write contention curve. Single-thread IO numbers from
/// `metadata.rs` × N would be the optimistic upper bound; this
/// shows what the SQLite write-serialization actually costs.
fn bench_lock_contention(c: &mut Criterion) {
    for &threads in &[1usize, 4, 8, 16, 32] {
        let mut group = c.benchmark_group(format!("lock_contention_{threads}t"));
        group.throughput(Throughput::Elements(threads as u64));
        group.sample_size(20);
        group.bench_function("acquire_release_unique", |b| {
            b.iter_custom(|iters| {
                let (_tmp, db) = fresh_db();
                let start = std::time::Instant::now();
                let mut handles = Vec::with_capacity(threads);
                for t in 0..threads {
                    let db = Arc::clone(&db);
                    handles.push(thread::spawn(move || {
                        for i in 0..iters {
                            let path = format!("Content/t{t}/lock{i}.umap");
                            let owner = format!("user-{t}");
                            db.acquire_lock("bench/repo", &path, &owner, "ws", "")
                                .unwrap()
                                .unwrap();
                            let ok = db
                                .release_lock("bench/repo", &path, &owner, false)
                                .unwrap();
                            assert!(ok);
                        }
                    }));
                }
                for h in handles {
                    h.join().unwrap();
                }
                start.elapsed()
            });
        });
        group.finish();
    }
}

/// N threads each scan `list_locks` against a populated table. Locks
/// are read-only here — a regression in the pool's WAL read path
/// would show up as throughput collapse vs single-thread.
fn bench_list_locks_concurrent(c: &mut Criterion) {
    for &threads in &[1usize, 4, 16] {
        let mut group = c.benchmark_group(format!("list_locks_concurrent_{threads}t"));
        group.throughput(Throughput::Elements(threads as u64));
        group.sample_size(20);
        group.bench_function("scan_500", |b| {
            let (_tmp, db) = fresh_db();
            for i in 0..500 {
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
            b.iter_custom(|iters| {
                let start = std::time::Instant::now();
                let mut handles = Vec::with_capacity(threads);
                for _ in 0..threads {
                    let db = Arc::clone(&db);
                    handles.push(thread::spawn(move || {
                        for _ in 0..iters {
                            let _ = db.list_locks("bench/repo", "", "").unwrap();
                        }
                    }));
                }
                for h in handles {
                    h.join().unwrap();
                }
                start.elapsed()
            });
        });
        group.finish();
    }
}

criterion_group!(benches, bench_lock_contention, bench_list_locks_concurrent);
criterion_main!(benches);
