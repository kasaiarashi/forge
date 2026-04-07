// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Network settings.
    #[serde(default)]
    pub server: ServerSection,

    /// Storage settings.
    #[serde(default)]
    pub storage: StorageSection,

    /// Per-repository overrides. Key = repo name.
    #[serde(default)]
    pub repos: std::collections::HashMap<String, RepoConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSection {
    /// Address to listen on.
    #[serde(default = "default_listen")]
    pub listen: String,

    /// Maximum size per gRPC message in bytes (not total push size).
    /// Individual objects larger than this are rejected.
    /// Push streams have no total size limit.
    #[serde(default = "default_max_upload", alias = "max_upload_size")]
    pub max_message_size: u64,

    /// Number of worker threads (0 = auto, uses all cores).
    #[serde(default)]
    pub workers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageSection {
    /// Base directory for all repository data.
    /// Each repo gets a subdirectory: <base>/<repo-name>/
    #[serde(default = "default_base_path")]
    pub base_path: PathBuf,

    /// Path to the SQLite metadata database.
    /// If relative, resolved from base_path.
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,
}

/// Per-repository configuration overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    /// Override storage path for this repo (absolute or relative to base_path).
    pub path: Option<PathBuf>,

    /// Optional description.
    pub description: Option<String>,
}

fn default_listen() -> String {
    "0.0.0.0:9876".into()
}
fn default_max_upload() -> u64 {
    256 * 1024 * 1024 // 256 MiB per message (objects are chunked, so this is generous)
}
fn default_base_path() -> PathBuf {
    PathBuf::from("./forge-data")
}
fn default_db_path() -> PathBuf {
    PathBuf::from("forge.db")
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            max_message_size: default_max_upload(),
            workers: 0,
        }
    }
}

impl Default for StorageSection {
    fn default() -> Self {
        Self {
            base_path: default_base_path(),
            db_path: default_db_path(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            server: ServerSection::default(),
            storage: StorageSection::default(),
            repos: std::collections::HashMap::new(),
        }
    }
}

impl ServerConfig {
    /// Load config from a TOML file. Returns default config if file doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;
        let config: ServerConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config: {}", path.display()))?;
        Ok(config)
    }

    /// Generate a default config file with comments.
    pub fn generate_default() -> String {
        r#"# Forge Server Configuration
# ========================

[server]
# Address and port to listen on.
listen = "0.0.0.0:9876"

# Maximum size per gRPC message in bytes. Default 256 MiB.
# This is per-message, NOT total push size — push streams are unlimited.
# Objects are chunked by FastCDC so individual messages are typically small.
max_message_size = 268435456

# Worker threads. 0 = auto (all CPU cores).
workers = 0

[storage]
# Base directory for all repository data.
# Each repo is stored in: <base_path>/repos/<repo-name>/objects/
base_path = "./forge-data"

# SQLite database path (relative to base_path, or absolute).
# Stores refs (branch tips) and lock metadata.
db_path = "forge.db"

# Per-repo overrides (optional).
# Useful for placing large repos on a different disk.
#
# [repos.my-game]
# path = "D:/fast-ssd/my-game"
# description = "Main game project"
#
# [repos.art-assets]
# path = "E:/large-hdd/art-assets"
# description = "Art asset repository (large storage)"
"#
        .to_string()
    }

    /// Resolve the objects directory for a given repo.
    pub fn repo_objects_path(&self, repo_name: &str) -> PathBuf {
        if let Some(repo) = self.repos.get(repo_name) {
            if let Some(ref path) = repo.path {
                if path.is_absolute() {
                    return path.join("objects");
                }
                return self.storage.base_path.join(path).join("objects");
            }
        }
        self.storage.base_path.join("repos").join(repo_name).join("objects")
    }

    /// Resolve the full database path.
    pub fn resolved_db_path(&self) -> PathBuf {
        if self.storage.db_path.is_absolute() {
            self.storage.db_path.clone()
        } else {
            self.storage.base_path.join(&self.storage.db_path)
        }
    }
}
