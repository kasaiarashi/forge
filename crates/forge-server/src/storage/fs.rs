// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use forge_core::store::chunk_store::ChunkStore;
use std::collections::HashMap;
use std::path::PathBuf;

/// Server-side filesystem storage with per-repo object directories.
pub struct FsStorage {
    base_path: PathBuf,
    /// Per-repo path overrides from config.
    repo_overrides: HashMap<String, PathBuf>,
}

impl FsStorage {
    pub fn new(base_path: PathBuf, repo_overrides: HashMap<String, PathBuf>) -> Self {
        std::fs::create_dir_all(&base_path).ok();
        Self {
            base_path,
            repo_overrides,
        }
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
    /// Respects per-repo path overrides from configuration.
    /// Get a ChunkStore for a specific repo's objects directory.
    /// Respects per-repo path overrides from configuration.
    /// Relative overrides are resolved against `base_path` (never its parent)
    /// and canonicalized to prevent path traversal.
    pub fn repo_store(&self, repo: &str) -> ChunkStore {
        let dir = if let Some(override_path) = self.repo_overrides.get(repo) {
            if override_path.is_absolute() {
                override_path.join("objects")
            } else {
                // Resolve relative to base_path, not its parent, to prevent traversal.
                let resolved = self.base_path.join(override_path).join("objects");
                // Canonicalize to collapse any ".." that might remain.
                resolved.canonicalize().unwrap_or(resolved)
            }
        } else {
            self.base_path.join(repo).join("objects")
        };
        std::fs::create_dir_all(&dir).ok();
        ChunkStore::new(dir)
    }
}
