// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use forge_core::store::chunk_store::ChunkStore;
use std::path::PathBuf;

/// Server-side filesystem storage with per-repo object directories.
pub struct FsStorage {
    base_path: PathBuf, // base directory, repos are subdirs
}

impl FsStorage {
    pub fn new(base_path: PathBuf) -> Self {
        std::fs::create_dir_all(&base_path).ok();
        Self { base_path }
    }

    /// Rename a repo directory from old name to new name.
    pub fn rename_repo(&self, old_name: &str, new_name: &str) -> std::io::Result<()> {
        let old_dir = self.base_path.join(old_name);
        let new_dir = self.base_path.join(new_name);
        if old_dir.exists() {
            std::fs::rename(&old_dir, &new_dir)?;
        }
        Ok(())
    }

    /// Delete a repo directory recursively.
    pub fn delete_repo(&self, name: &str) -> std::io::Result<()> {
        let dir = self.base_path.join(name);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    /// Get a ChunkStore for a specific repo's objects directory.
    pub fn repo_store(&self, repo: &str) -> ChunkStore {
        let dir = self.base_path.join(repo).join("objects");
        std::fs::create_dir_all(&dir).ok();
        ChunkStore::new(dir)
    }
}
