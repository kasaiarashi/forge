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

    /// Actions/workflow engine settings.
    #[serde(default)]
    pub actions: ActionsSection,
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

    /// Optional TLS configuration. When present, the gRPC server terminates
    /// TLS using rustls; when absent, it speaks plaintext h2c and should
    /// only be bound to loopback.
    #[serde(default)]
    pub tls: Option<TlsConfig>,
}

/// TLS settings for the gRPC server.
///
/// Two modes:
/// - **Manual**: supply `cert_path` and `key_path` pointing at real PEM
///   files (e.g. from an ACME client).
/// - **Auto-generate**: set `auto_generate = true` and leave `cert_path`
///   and `key_path` at their defaults. On first start, forge-server mints
///   a local CA and a leaf certificate covering `hostnames` + loopback,
///   writes them under `<base_path>/certs/`, and reuses them on every
///   subsequent start. Use `forge trust` from client machines to pin the
///   CA.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// PEM-encoded certificate chain (leaf first). Defaults to
    /// `<base_path>/certs/server.crt`.
    #[serde(default)]
    pub cert_path: Option<PathBuf>,

    /// PEM-encoded private key matching the certificate. Defaults to
    /// `<base_path>/certs/server.key`.
    #[serde(default)]
    pub key_path: Option<PathBuf>,

    /// When true, generate a CA + leaf on first start if the files don't
    /// exist yet. When false, missing files are a startup error.
    #[serde(default)]
    pub auto_generate: bool,

    /// DNS names / IP addresses to encode into the leaf cert's
    /// `subjectAltName` extension. `localhost`, `127.0.0.1`, and `::1` are
    /// always added implicitly. Ignored when `auto_generate` is false.
    #[serde(default)]
    pub hostnames: Vec<String>,
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

/// Actions/workflow engine settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionsSection {
    /// Enable the actions engine. Default **false** because workflow steps
    /// run as arbitrary shell commands on the server host with the
    /// forge-server process's privileges.
    #[serde(default = "default_false")]
    pub enabled: bool,

    /// Base directory for artifacts storage (relative to base_path, or absolute).
    #[serde(default = "default_artifacts_path")]
    pub artifacts_path: PathBuf,

    /// Base directory for temporary workspace checkouts.
    #[serde(default = "default_workspaces_path")]
    pub workspaces_path: PathBuf,

    /// Maximum concurrent workflow runs across all repos.
    #[serde(default = "default_max_runs")]
    pub max_concurrent_runs: usize,

    /// Execution environment: "native" (default) or "container" (stubbed).
    #[serde(default = "default_executor")]
    pub executor: String,
}

fn default_false() -> bool { false }
fn default_artifacts_path() -> PathBuf { PathBuf::from("artifacts") }
fn default_workspaces_path() -> PathBuf { PathBuf::from("workspaces") }
fn default_max_runs() -> usize { 1 }
fn default_executor() -> String { "native".into() }

impl Default for ActionsSection {
    fn default() -> Self {
        Self {
            enabled: default_false(),
            artifacts_path: default_artifacts_path(),
            workspaces_path: default_workspaces_path(),
            max_concurrent_runs: default_max_runs(),
            executor: default_executor(),
        }
    }
}

fn default_listen() -> String {
    "127.0.0.1:9876".into()
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
            tls: None,
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
            actions: ActionsSection::default(),
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
# Address and port to listen on. Bind to 0.0.0.0 to expose the server on
# the network — the default [server.tls] block below keeps the connection
# encrypted regardless.
listen = "0.0.0.0:9876"

# Maximum size per gRPC message in bytes. Default 256 MiB.
# This is per-message, NOT total push size — push streams are unlimited.
# Objects are chunked by FastCDC so individual messages are typically small.
max_message_size = 268435456

# Worker threads. 0 = auto (all CPU cores).
workers = 0

# TLS is ON by default. On first start, forge-server generates a local CA
# under <storage.base_path>/certs/, mints a leaf cert for the hostnames
# listed below, and prints the CA's SHA-256 fingerprint to the logs.
#
# Distribute to clients with:
#   forge trust https://<this-server>:9876
#
# Already have a real cert (Let's Encrypt, corporate CA)? Set `auto_generate
# = false` and point `cert_path` + `key_path` at your PEM files.
[server.tls]
auto_generate = true
# hostnames = ["forge.example.com", "10.0.0.5"]
# cert_path = "./certs/server.crt"
# key_path  = "./certs/server.key"

[storage]
# Base directory for all repository data.
# Each repo is stored in: <base_path>/repos/<repo-name>/objects/
base_path = "./forge-data"

# SQLite database path (relative to base_path, or absolute).
# Stores refs (branch tips) and lock metadata.
db_path = "forge.db"

[actions]
# DANGER: workflow steps run as arbitrary shell commands on THIS machine
# as the forge-server process user. Anyone with a repo:admin role on any
# repo can author a workflow that executes code on the host. The engine is
# DISABLED by default — enable it only in dedicated, isolated deployments.
enabled = false

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

# Authentication is always on. Create users with:
#   forge-server user add --admin <username>
# or via the web setup wizard at /setup on first run.
"#
        .to_string()
    }

    /// Resolve the full database path.
    pub fn resolved_db_path(&self) -> PathBuf {
        if self.storage.db_path.is_absolute() {
            self.storage.db_path.clone()
        } else {
            self.storage.base_path.join(&self.storage.db_path)
        }
    }

    /// Resolve the artifacts directory.
    pub fn resolved_artifacts_path(&self) -> PathBuf {
        if self.actions.artifacts_path.is_absolute() {
            self.actions.artifacts_path.clone()
        } else {
            self.storage.base_path.join(&self.actions.artifacts_path)
        }
    }

    /// Resolve the workspaces directory.
    pub fn resolved_workspaces_path(&self) -> PathBuf {
        if self.actions.workspaces_path.is_absolute() {
            self.actions.workspaces_path.clone()
        } else {
            self.storage.base_path.join(&self.actions.workspaces_path)
        }
    }
}
