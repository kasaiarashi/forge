// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Live step-log broadcast hub.
//!
//! Each running step publishes stdout/stderr chunks (UTF-8 with lossy
//! replacement at the capture boundary) to a per-run broadcast channel.
//! Readers — gRPC `StreamStepLogs` clients, future agent log forwarders —
//! subscribe lazily and receive everything produced after they join.
//!
//! Catch-up for the pre-subscribe window is handled by the gRPC handler:
//! it reads the DB log row first, then subscribes. A small amount of
//! overlap is tolerable (the client gets a duplicate suffix / the live
//! stream gets a duplicate prefix); clients dedupe on `step_id` + byte
//! offset if they care.

use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::broadcast;

/// One chunk of log output. `step_id` lets subscribers filter when a run
/// is multi-step. `is_final` is set on the terminating chunk so readers
/// know they can close cleanly instead of waiting for a timeout.
#[derive(Debug, Clone)]
pub struct LogChunk {
    pub run_id: i64,
    pub step_id: i64,
    pub data: Vec<u8>,
    pub is_final: bool,
}

pub struct LogHub {
    channels: Mutex<HashMap<i64, broadcast::Sender<LogChunk>>>,
    capacity: usize,
}

impl LogHub {
    pub fn new() -> Self {
        Self {
            channels: Mutex::new(HashMap::new()),
            // 256 chunks buffered before slow subscribers drop frames.
            // Subscribers that lag past that catch up via DB re-read.
            capacity: 256,
        }
    }

    /// Get or create a sender for `run_id`. Always returns a live sender
    /// even when there are zero subscribers — writers should never care.
    pub fn sender(&self, run_id: i64) -> broadcast::Sender<LogChunk> {
        let mut guard = self.channels.lock().expect("log hub poisoned");
        guard
            .entry(run_id)
            .or_insert_with(|| broadcast::channel(self.capacity).0)
            .clone()
    }

    /// Subscribe to live chunks for `run_id`. Returns a receiver that yields
    /// only chunks sent *after* subscribe — do the DB catch-up before
    /// calling this.
    pub fn subscribe(&self, run_id: i64) -> broadcast::Receiver<LogChunk> {
        self.sender(run_id).subscribe()
    }

    /// Tear down the channel once a run finishes so the HashMap doesn't
    /// grow without bound. Subscribers still holding a Receiver will see
    /// a `Closed` error on their next recv.
    pub fn close(&self, run_id: i64) {
        let mut guard = self.channels.lock().expect("log hub poisoned");
        guard.remove(&run_id);
    }
}

impl Default for LogHub {
    fn default() -> Self {
        Self::new()
    }
}
