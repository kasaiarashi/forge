// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Reconstruct a repo's tree at a given commit into a temporary directory.

use anyhow::{Context, Result};
use forge_core::hash::ForgeHash;
use forge_core::object::tree::EntryKind;
use forge_core::store::object_store::ObjectStore;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Checkout a repo's tree at a given commit to a directory.
/// Returns the path to the workspace.
pub fn checkout(
    object_store: &ObjectStore,
    commit_hash: &ForgeHash,
    workspace_dir: &Path,
) -> Result<PathBuf> {
    std::fs::create_dir_all(workspace_dir)?;
    // On Unix, lock the workspace dir to the server user. This is the
    // short-term hardening for the in-process runner — once Phase 2 lands
    // an external agent with a dedicated OS user, that becomes the real
    // isolation boundary. Windows leaves ACLs to installer inheritance.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(workspace_dir)?.permissions();
        perms.set_mode(0o700);
        let _ = std::fs::set_permissions(workspace_dir, perms);
    }

    let snapshot = object_store
        .get_snapshot(commit_hash)
        .with_context(|| format!("Failed to read commit {}", commit_hash.short()))?;

    write_tree(object_store, &snapshot.tree, workspace_dir)?;
    debug!("Checked out {} to {}", commit_hash.short(), workspace_dir.display());
    Ok(workspace_dir.to_path_buf())
}

fn write_tree(
    object_store: &ObjectStore,
    tree_hash: &ForgeHash,
    dir: &Path,
) -> Result<()> {
    let tree = object_store.get_tree(tree_hash)?;

    for entry in &tree.entries {
        let entry_path = dir.join(&entry.name);

        match entry.kind {
            EntryKind::Directory => {
                std::fs::create_dir_all(&entry_path)?;
                write_tree(object_store, &entry.hash, &entry_path)?;
            }
            EntryKind::File | EntryKind::Symlink => {
                let data = object_store.get_blob_data(&entry.hash)?;
                if let Some(parent) = entry_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&entry_path, &data)?;
            }
        }
    }

    Ok(())
}

/// Clean up a workspace directory.
pub fn cleanup(workspace_dir: &Path) {
    if workspace_dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(workspace_dir) {
            tracing::warn!("Failed to clean up workspace {}: {}", workspace_dir.display(), e);
        }
    }
}
