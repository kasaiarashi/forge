// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Live lock-event broadcast hub (Phase 4d).
//!
//! Parallels [`super::logs::LogHub`]: one [`tokio::sync::broadcast`]
//! channel per repo, allocated lazily. `AcquireLock`/`ReleaseLock`
//! handlers publish events; `StreamLockEvents` subscribers receive
//! them in real time.
//!
//! Kills the Unreal Engine plugin's polling loop. Today every editor
//! instance calls `ListLocks` on a timer (originally via `forge`
//! subprocess, now via [`forge-ffi::forge_lock_list_json`] after
//! Phase 4b). With this hub the plugin subscribes once at module
//! init and handles events as they land — zero extra traffic until
//! something actually changes.
//!
//! Slow-subscriber policy: the channel capacity is small (256 events
//! per repo). Subscribers that fall behind see a `Lagged(n)` from
//! `broadcast::Receiver::recv` — the plugin treats it as "I missed
//! some events, reset my view via ListLocks and re-subscribe" so
//! overflow degrades gracefully rather than silently corrupting state.

use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::broadcast;

use forge_proto::forge::{lock_event::Kind as LockEventKind, LockEvent, LockInfo};

/// Monotonic sequence source, shared across every repo. A single
/// counter keeps the per-event `seq` strictly increasing even when
/// subscribers follow multiple repos, which simplifies client-side
/// dedup (repo + seq is unique).
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_seq() -> u64 {
    NEXT_SEQ.fetch_add(1, Ordering::Relaxed)
}

pub struct LockEventHub {
    channels: Mutex<HashMap<String, broadcast::Sender<LockEvent>>>,
    capacity: usize,
}

impl LockEventHub {
    pub fn new() -> Self {
        Self {
            channels: Mutex::new(HashMap::new()),
            capacity: 256,
        }
    }

    fn sender(&self, repo: &str) -> broadcast::Sender<LockEvent> {
        let mut guard = self.channels.lock().expect("lock hub poisoned");
        guard
            .entry(repo.to_string())
            .or_insert_with(|| broadcast::channel(self.capacity).0)
            .clone()
    }

    /// Subscribe to future events on `repo`. The server-side RPC
    /// handler first emits a SNAPSHOT set from `MetadataDb::list_locks`
    /// to give the subscriber a complete current-state view, then
    /// forwards everything the hub publishes afterward.
    pub fn subscribe(&self, repo: &str) -> broadcast::Receiver<LockEvent> {
        self.sender(repo).subscribe()
    }

    pub fn publish_acquire(&self, repo: &str, info: LockInfo) {
        let _ = self.sender(repo).send(LockEvent {
            kind: LockEventKind::Acquire as i32,
            info: Some(info),
            seq: next_seq(),
        });
    }

    pub fn publish_release(&self, repo: &str, info: LockInfo) {
        let _ = self.sender(repo).send(LockEvent {
            kind: LockEventKind::Release as i32,
            info: Some(info),
            seq: next_seq(),
        });
    }
}

impl Default for LockEventHub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(path: &str, owner: &str) -> LockInfo {
        LockInfo {
            path: path.to_string(),
            owner: owner.to_string(),
            workspace_id: "ws".into(),
            reason: String::new(),
            created_at: 0,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscriber_receives_events_on_its_repo_only() {
        let hub = LockEventHub::new();
        let mut rx_a = hub.subscribe("repo/a");
        let mut rx_b = hub.subscribe("repo/b");

        hub.publish_acquire("repo/a", mk("Content/Foo.uasset", "alice"));
        hub.publish_release("repo/a", mk("Content/Foo.uasset", "alice"));
        hub.publish_acquire("repo/b", mk("Content/Bar.uasset", "bob"));

        // repo/a stream: acquire + release only.
        let e1 = rx_a.recv().await.unwrap();
        assert_eq!(e1.kind, LockEventKind::Acquire as i32);
        assert_eq!(e1.info.as_ref().unwrap().owner, "alice");
        let e2 = rx_a.recv().await.unwrap();
        assert_eq!(e2.kind, LockEventKind::Release as i32);
        assert!(e2.seq > e1.seq, "seq must be strictly monotonic");

        // repo/b stream: the unrelated bob event.
        let e3 = rx_b.recv().await.unwrap();
        assert_eq!(e3.info.as_ref().unwrap().owner, "bob");
    }
}
