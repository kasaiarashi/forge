use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::ForgeError;
use crate::hash::ForgeHash;
use crate::object::snapshot::Author;
use crate::store::object_store::ObjectStore;

/// The `.forge` directory name.
pub const FORGE_DIR: &str = ".forge";

/// Current HEAD reference.
#[derive(Debug, Clone)]
pub enum HeadRef {
    /// Points to a branch by name.
    Branch(String),
    /// Detached, pointing directly at a snapshot hash.
    Detached(ForgeHash),
}

/// How the workspace handles concurrent edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowMode {
    /// Perforce-style: binary files must be locked before editing.
    /// Conflicts are prevented — only the lock holder can modify a file.
    Lock,
    /// Git-style: anyone edits freely, conflicts resolved at push time
    /// by diffing and choosing which user's version to keep.
    Merge,
}

impl Default for WorkflowMode {
    fn default() -> Self {
        Self::Lock
    }
}

impl std::fmt::Display for WorkflowMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lock => write!(f, "lock"),
            Self::Merge => write!(f, "merge"),
        }
    }
}

/// A named remote server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Remote {
    /// Display name (e.g., "origin", "staging", "production").
    pub name: String,
    /// Server URL (e.g., "http://localhost:9876", "https://forge.mycompany.com:9876").
    pub url: String,
}

/// Workspace configuration stored in `.forge/config.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// The user identity.
    pub user: Author,
    /// Unique ID for this workspace instance.
    pub workspace_id: String,
    /// Workflow mode: "lock" (perforce-style) or "merge" (git-style).
    #[serde(default)]
    pub workflow: WorkflowMode,
    /// Repository name on the server (like GitHub's "owner/repo").
    #[serde(default)]
    pub repo: String,
    /// Named remotes (like git remotes). First one is the default.
    #[serde(default)]
    pub remotes: Vec<Remote>,
    /// Patterns for auto-locking in lock mode (e.g., ["*.uasset", "*.umap"]).
    #[serde(default)]
    pub auto_lock_patterns: Vec<String>,

    // Legacy field — kept for backwards compatibility during migration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
}

impl WorkspaceConfig {
    /// Get the default remote (first in the list, or legacy server_url).
    pub fn default_remote(&self) -> Option<&Remote> {
        self.remotes.first()
    }

    /// Get the URL of the default remote.
    pub fn default_remote_url(&self) -> Option<&str> {
        self.remotes
            .first()
            .map(|r| r.url.as_str())
            .or(self.server_url.as_deref())
    }

    /// Get a remote by name.
    pub fn get_remote(&self, name: &str) -> Option<&Remote> {
        self.remotes.iter().find(|r| r.name == name)
    }

    /// Add a remote. Returns error if name already exists.
    pub fn add_remote(&mut self, name: String, url: String) -> Result<(), ForgeError> {
        if self.remotes.iter().any(|r| r.name == name) {
            return Err(ForgeError::Other(format!(
                "Remote '{}' already exists",
                name
            )));
        }
        self.remotes.push(Remote { name, url });
        Ok(())
    }

    /// Remove a remote by name.
    pub fn remove_remote(&mut self, name: &str) -> Result<(), ForgeError> {
        let len = self.remotes.len();
        self.remotes.retain(|r| r.name != name);
        if self.remotes.len() == len {
            return Err(ForgeError::Other(format!(
                "Remote '{}' not found",
                name
            )));
        }
        Ok(())
    }

    /// Rename a remote.
    pub fn rename_remote(&mut self, old: &str, new: &str) -> Result<(), ForgeError> {
        if self.remotes.iter().any(|r| r.name == new) {
            return Err(ForgeError::Other(format!(
                "Remote '{}' already exists",
                new
            )));
        }
        let remote = self
            .remotes
            .iter_mut()
            .find(|r| r.name == old)
            .ok_or_else(|| ForgeError::Other(format!("Remote '{}' not found", old)))?;
        remote.name = new.to_string();
        Ok(())
    }

    /// Set the URL of an existing remote.
    pub fn set_remote_url(&mut self, name: &str, url: String) -> Result<(), ForgeError> {
        let remote = self
            .remotes
            .iter_mut()
            .find(|r| r.name == name)
            .ok_or_else(|| ForgeError::Other(format!("Remote '{}' not found", name)))?;
        remote.url = url;
        Ok(())
    }
}

/// Represents a Forge workspace rooted at a project directory.
pub struct Workspace {
    /// The root of the user's project (parent of .forge/).
    pub root: PathBuf,
    /// The object store.
    pub object_store: ObjectStore,
}

impl Workspace {
    /// The .forge directory path.
    pub fn forge_dir(&self) -> PathBuf {
        self.root.join(FORGE_DIR)
    }

    /// Discover a workspace by walking up from `start`.
    pub fn discover(start: &Path) -> Result<Self, ForgeError> {
        let mut current = start.to_path_buf();
        loop {
            let forge_dir = current.join(FORGE_DIR);
            if forge_dir.is_dir() {
                let objects_dir = forge_dir.join("objects");
                return Ok(Self {
                    root: current,
                    object_store: ObjectStore::new(objects_dir),
                });
            }
            if !current.pop() {
                return Err(ForgeError::NotAWorkspace);
            }
        }
    }

    /// Initialize a new workspace at `root`.
    pub fn init(root: &Path, user: Author) -> Result<Self, ForgeError> {
        let forge_dir = root.join(FORGE_DIR);
        if forge_dir.exists() {
            return Err(ForgeError::AlreadyInitialized(
                root.display().to_string(),
            ));
        }

        // Create directory structure.
        std::fs::create_dir_all(forge_dir.join("objects"))?;
        std::fs::create_dir_all(forge_dir.join("refs").join("heads"))?;
        std::fs::create_dir_all(forge_dir.join("refs").join("remotes"))?;
        std::fs::create_dir_all(forge_dir.join("locks"))?;

        // Write HEAD pointing to main branch (atomic).
        atomic_write(&forge_dir.join("HEAD"), b"ref: refs/heads/main\n")?;

        // Write initial branch ref (zero hash = no snapshots yet, atomic).
        atomic_write(
            &forge_dir.join("refs").join("heads").join("main"),
            ForgeHash::ZERO.to_hex().as_bytes(),
        )?;

        // Write config.
        let config = WorkspaceConfig {
            user,
            workspace_id: uuid::Uuid::new_v4().to_string(),
            workflow: WorkflowMode::default(),
            repo: String::new(),
            remotes: vec![],
            auto_lock_patterns: vec![
                "*.uasset".into(),
                "*.umap".into(),
                "*.uexp".into(),
                "*.ubulk".into(),
            ],
            server_url: None,
        };
        let config_json = serde_json::to_string_pretty(&config)
            .map_err(|e| ForgeError::Serialization(e.to_string()))?;
        atomic_write(&forge_dir.join("config.json"), config_json.as_bytes())?;

        // Write empty index.
        let index = crate::index::Index::default();
        index.save(&forge_dir.join("index"))?;

        let objects_dir = forge_dir.join("objects");
        Ok(Self {
            root: root.to_path_buf(),
            object_store: ObjectStore::new(objects_dir),
        })
    }

    /// Read the current HEAD reference.
    pub fn head(&self) -> Result<HeadRef, ForgeError> {
        let head_path = self.forge_dir().join("HEAD");
        let content = std::fs::read_to_string(&head_path)?;
        let content = content.trim();

        if let Some(ref_name) = content.strip_prefix("ref: ") {
            // Extract branch name from "refs/heads/<name>".
            let branch = ref_name
                .strip_prefix("refs/heads/")
                .unwrap_or(ref_name)
                .to_string();
            Ok(HeadRef::Branch(branch))
        } else {
            let hash = ForgeHash::from_hex(content)?;
            Ok(HeadRef::Detached(hash))
        }
    }

    /// Set the HEAD reference (atomic write).
    pub fn set_head(&self, head: &HeadRef) -> Result<(), ForgeError> {
        let head_path = self.forge_dir().join("HEAD");
        let content = match head {
            HeadRef::Branch(name) => format!("ref: refs/heads/{}\n", name),
            HeadRef::Detached(hash) => format!("{}\n", hash.to_hex()),
        };
        atomic_write(&head_path, content.as_bytes())?;
        Ok(())
    }

    /// Get the snapshot hash that the current HEAD points to.
    pub fn head_snapshot(&self) -> Result<ForgeHash, ForgeError> {
        match self.head()? {
            HeadRef::Branch(name) => self.get_branch_tip(&name),
            HeadRef::Detached(hash) => Ok(hash),
        }
    }

    /// Resolve a string to a ForgeHash. Accepts: full hex hash, short hex prefix (>=6 chars), or branch name.
    pub fn resolve_ref(&self, s: &str) -> Result<ForgeHash, ForgeError> {
        // Try as branch name first.
        if let Ok(hash) = self.get_branch_tip(s) {
            return Ok(hash);
        }
        // Try as full hex hash.
        if s.len() == 64 {
            return ForgeHash::from_hex(s);
        }
        // Try as short hash prefix (scan object store).
        if s.len() >= 6 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            let shard = &s[..2];
            let rest_prefix = &s[2..];
            let shard_dir = self.forge_dir().join("objects").join(shard);
            if shard_dir.exists() {
                let mut matches = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&shard_dir) {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.file_name().to_str() {
                            if name.starts_with(rest_prefix) {
                                let full_hex = format!("{}{}", shard, name);
                                if let Ok(hash) = ForgeHash::from_hex(&full_hex) {
                                    matches.push(hash);
                                }
                            }
                        }
                    }
                }
                return match matches.len() {
                    0 => Err(ForgeError::ObjectNotFound(s.to_string())),
                    1 => Ok(matches[0]),
                    _ => Err(ForgeError::Other(format!(
                        "ambiguous short hash '{}' matches {} objects", s, matches.len()
                    ))),
                };
            }
        }
        Err(ForgeError::InvalidHash(format!("cannot resolve '{}'", s)))
    }

    /// Get the snapshot hash at the tip of a branch.
    pub fn get_branch_tip(&self, branch: &str) -> Result<ForgeHash, ForgeError> {
        let ref_path = self.forge_dir().join("refs").join("heads").join(branch);
        if !ref_path.exists() {
            return Err(ForgeError::BranchNotFound(branch.to_string()));
        }
        let content = std::fs::read_to_string(&ref_path)?;
        ForgeHash::from_hex(content.trim())
    }

    /// Update the tip of a branch (atomic write).
    pub fn set_branch_tip(&self, branch: &str, hash: &ForgeHash) -> Result<(), ForgeError> {
        let ref_path = self.forge_dir().join("refs").join("heads").join(branch);
        if let Some(parent) = ref_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        atomic_write(&ref_path, hash.to_hex().as_bytes())?;
        Ok(())
    }

    /// List all branches (supports nested names like `feature/foo`).
    pub fn list_branches(&self) -> Result<Vec<String>, ForgeError> {
        let heads_dir = self.forge_dir().join("refs").join("heads");
        let mut branches = Vec::new();
        if heads_dir.exists() {
            Self::collect_branches(&heads_dir, "", &mut branches)?;
        }
        branches.sort();
        Ok(branches)
    }

    fn collect_branches(
        dir: &std::path::Path,
        prefix: &str,
        out: &mut Vec<String>,
    ) -> Result<(), ForgeError> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
            let full = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", prefix, name)
            };
            if entry.file_type()?.is_file() {
                out.push(full);
            } else if entry.file_type()?.is_dir() {
                Self::collect_branches(&entry.path(), &full, out)?;
            }
        }
        Ok(())
    }

    /// Get the current branch name (if HEAD points to a branch).
    pub fn current_branch(&self) -> Result<Option<String>, ForgeError> {
        match self.head()? {
            HeadRef::Branch(name) => Ok(Some(name)),
            HeadRef::Detached(_) => Ok(None),
        }
    }

    /// Load workspace config.
    pub fn config(&self) -> Result<WorkspaceConfig, ForgeError> {
        let config_path = self.forge_dir().join("config.json");
        let content = std::fs::read_to_string(&config_path)?;
        let config: WorkspaceConfig = serde_json::from_str(&content)
            .map_err(|e| ForgeError::Serialization(e.to_string()))?;
        Ok(config)
    }

    /// Save workspace config back to disk (atomic write).
    pub fn save_config(&self, config: &WorkspaceConfig) -> Result<(), ForgeError> {
        let config_path = self.forge_dir().join("config.json");
        let json = serde_json::to_string_pretty(config)
            .map_err(|e| ForgeError::Serialization(e.to_string()))?;
        atomic_write(&config_path, json.as_bytes())?;
        Ok(())
    }
}

/// Write data atomically: write to a temp file, then rename over the target.
/// This prevents partial/corrupt writes on crash or power loss.
pub fn atomic_write(path: &std::path::Path, data: &[u8]) -> Result<(), ForgeError> {
    // Use a unique temp name to avoid collisions with concurrent writers.
    let unique = std::process::id();
    let tmp = path.with_extension(format!("tmp.{}", unique));
    std::fs::write(&tmp, data)?;

    #[cfg(not(target_os = "windows"))]
    {
        std::fs::rename(&tmp, path)?;
    }

    #[cfg(target_os = "windows")]
    {
        // On Windows, try rename first (works if target doesn't exist).
        // If it fails because target exists, remove target then retry.
        if std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(path);
            if let Err(e) = std::fs::rename(&tmp, path) {
                // Clean up temp on failure.
                let _ = std::fs::remove_file(&tmp);
                return Err(e.into());
            }
        }
    }

    Ok(())
}
