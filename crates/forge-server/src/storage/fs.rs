// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use forge_core::store::chunk_store::ChunkStore;
use std::path::PathBuf;

/// Server-side filesystem storage wrapping the core ChunkStore.
pub struct FsStorage {
    pub store: ChunkStore,
}

impl FsStorage {
    pub fn new(objects_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&objects_dir).ok();
        Self {
            store: ChunkStore::new(objects_dir),
        }
    }
}
