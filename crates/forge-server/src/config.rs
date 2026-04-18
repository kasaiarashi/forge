// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

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

    /// Artifact storage settings (run outputs, release assets).
    #[serde(default)]
    pub artifacts: ArtifactsSection,

    /// Logging + audit output.
    #[serde(default)]
    pub logging: LoggingSection,

    /// Push / object-transfer limits.
    #[serde(default)]
    pub limits: LimitsSection,

    /// Metadata database backend and pool tuning.
    #[serde(default)]
    pub database: DatabaseSection,

    /// Live object-store backend. Phase 3b.4 — FS (default) stores
    /// every repo's objects under `<base_path>/repos/<repo>/objects/`;
    /// `backend = "s3"` publishes them to an S3-compatible bucket
    /// (MinIO, AWS S3, Ceph RGW) while staging stays on local disk.
    #[serde(default)]
    pub objects: ObjectsSection,

    /// `/metrics` + `/healthz` + `/readyz` HTTP listener (Phase 7).
    /// Separate port so a scraper can hit the server without needing
    /// the gRPC TLS trust chain.
    #[serde(default)]
    pub metrics: MetricsSection,
}

/// `[metrics]` block — controls the Prometheus + health HTTP listener.
///
/// The listener speaks plain HTTP (no TLS) because scrape endpoints
/// traditionally ride on the internal network — a sidecar Prometheus
/// or a k8s liveness probe doesn't want to learn the forge CA.
/// Binding to loopback by default keeps it off the wire for operators
/// who haven't thought about scraping yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSection {
    /// Turn the listener on. Default true — exposing health +
    /// a handful of gauges costs nothing.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Listen address for the scrape endpoints. Default
    /// `127.0.0.1:9877` so it's reachable by a local scraper but
    /// not the open network. Operators running a Prometheus
    /// sidecar on the same host point it here.
    #[serde(default = "default_metrics_listen")]
    pub listen: String,
}

fn default_true() -> bool {
    true
}
fn default_metrics_listen() -> String {
    "127.0.0.1:9877".into()
}

impl Default for MetricsSection {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            listen: default_metrics_listen(),
        }
    }
}

/// `[objects]` block — picks the live object-store backend. Staging
/// is always on local disk (see `S3RepoStorage` for why); this
/// selector only controls where committed objects land.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ObjectsSection {
    /// `"fs"` (default) keeps the pre-Phase-3b.4 local layout. `"s3"`
    /// publishes to the `[objects.s3]` bucket; requires the
    /// `s3-objects` Cargo feature at compile time (else the server
    /// bails loudly at startup).
    #[serde(default = "default_objects_backend")]
    pub backend: String,

    /// S3 connection settings. Only consulted when
    /// `backend = "s3"`. The shape mirrors `[artifacts.s3]` on
    /// purpose so an operator who already configured artifact
    /// storage can copy-paste.
    #[serde(default)]
    pub s3: ObjectsS3,
}

fn default_objects_backend() -> String {
    "fs".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ObjectsS3 {
    /// Required. Bucket name (NOT URI — use `endpoint_url` for
    /// non-AWS endpoints).
    #[serde(default)]
    pub bucket: String,
    /// Optional AWS region. Empty → `us-east-1` (MinIO ignores
    /// it but the SDK still wants something non-empty).
    #[serde(default)]
    pub region: String,
    /// Custom endpoint URL for MinIO / Ceph RGW. Empty → AWS
    /// default endpoint resolver.
    #[serde(default)]
    pub endpoint_url: String,
    /// Optional key prefix that wraps every repo's tree. Lets
    /// multiple forge-servers share a bucket — each gets its
    /// own prefix, per-repo subtrees land as
    /// `{prefix}{repo}/objects/<ab>/<rest>`.
    #[serde(default)]
    pub prefix: String,
    /// Static access key. Empty → fall back to the default AWS
    /// credential chain (env / profile / IMDS).
    #[serde(default)]
    pub access_key_id: String,
    /// Static secret key. Empty → default chain.
    #[serde(default)]
    pub secret_access_key: String,
    /// Force path-style URLs. Required for MinIO's standard
    /// deployment; MUST be `false` for AWS virtual-hosted style.
    #[serde(default)]
    pub path_style: bool,
}

/// Metadata backend configuration. The default is SQLite with pooled
/// connections so the server starts with zero external dependencies but
/// no longer serialises every write on a single Mutex. Postgres lands
/// behind the same section in Phase 2b.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseSection {
    /// Backend selector. `"sqlite"` (default) uses the path at
    /// `storage.db_path`. `"postgres"` is reserved for Phase 2b and
    /// consults `url`.
    #[serde(default = "default_db_backend")]
    pub backend: String,

    /// Connection string. Ignored for the SQLite backend (uses
    /// `storage.db_path`); required for Postgres.
    #[serde(default)]
    pub url: String,

    /// Max pool size. SQLite uses WAL so many readers are cheap; a single
    /// writer still serialises on the DB file, but `BEGIN IMMEDIATE`
    /// prevents the write-starvation that deferred txns suffered under
    /// the old Mutex design. Default 16 is generous for a single-host
    /// deploy.
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// How long (ms) a pooled connection waits for the SQLite write lock
    /// before erroring. `busy_timeout` inside SQLite — not an r2d2 wait.
    /// 5 s is enough to ride out a batch-push burst; longer risks
    /// gRPC-side client timeouts.
    #[serde(default = "default_busy_timeout_ms")]
    pub busy_timeout_ms: u64,
}

fn default_db_backend() -> String {
    "sqlite".into()
}
fn default_max_connections() -> u32 {
    16
}
fn default_busy_timeout_ms() -> u64 {
    5_000
}

impl Default for DatabaseSection {
    fn default() -> Self {
        Self {
            backend: default_db_backend(),
            url: String::new(),
            max_connections: default_max_connections(),
            busy_timeout_ms: default_busy_timeout_ms(),
        }
    }
}

/// Size + time ceilings applied to push and upload-session handling.
/// Raising the per-object cap costs nothing on disk — objects stream
/// through staging — but an excessively high ceiling lets a rogue client
/// exhaust server disk, so keep the default high enough for UE cooked
/// builds (a few GiB) but below the full disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsSection {
    /// Per-object size cap (bytes). Any single object whose total size
    /// exceeds this is rejected at chunk time. Default 16 GiB — large
    /// enough for typical UE5 cooked `.pak` files, small enough that a
    /// single bug can't fill a production disk in one push.
    #[serde(default = "default_max_object_size")]
    pub max_object_size: u64,

    /// Seconds an upload session lives before the sweeper reclaims its
    /// staging directory. Resumed pushes (Phase 3) push expires_at
    /// forward as they land chunks.
    #[serde(default = "default_upload_ttl")]
    pub upload_session_ttl_seconds: i64,
}

fn default_max_object_size() -> u64 {
    16 * 1024 * 1024 * 1024 // 16 GiB
}
fn default_upload_ttl() -> i64 {
    60 * 60 // 1 hour
}

impl Default for LimitsSection {
    fn default() -> Self {
        Self {
            max_object_size: default_max_object_size(),
            upload_session_ttl_seconds: default_upload_ttl(),
        }
    }
}

/// Logging configuration. All fields optional; safe defaults apply.
///
/// File sinks use daily rotation under `dir`. The audit sink emits only
/// events whose tracing target is `audit` (see `observability::audit`);
/// the application sink captures everything else at `level`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingSection {
    /// Minimum level for the application log sink. Accepts any
    /// `tracing_subscriber::EnvFilter` directive (e.g. `"info"`,
    /// `"forge_server=debug,tonic=warn"`). Overridden by the `RUST_LOG`
    /// environment variable when set.
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Output format. `"text"` = human-readable (default); `"json"` =
    /// one JSON object per line, suitable for log shippers.
    #[serde(default = "default_log_format")]
    pub format: String,

    /// Directory for rotating log files. Relative paths resolve against
    /// `storage.base_path`. When empty, file logging is disabled — logs
    /// go to stdout only, preserving the pre-audit behaviour.
    #[serde(default)]
    pub dir: PathBuf,

    /// Also mirror the application log to stdout. Off by default when a
    /// `dir` is configured — operators tailing a file don't also want the
    /// console cluttered. Always on when `dir` is empty, regardless of
    /// this flag.
    #[serde(default)]
    pub stdout: bool,
}

fn default_log_level() -> String {
    "info".into()
}
fn default_log_format() -> String {
    "text".into()
}

impl Default for LoggingSection {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
            dir: PathBuf::new(),
            stdout: false,
        }
    }
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

    /// TLS configuration. **TLS is on by default.** Even if the operator's
    /// config file is missing the `[server.tls]` section entirely,
    /// `TlsConfig::default()` produces `enabled = true` + `auto_generate
    /// = true`, so a fresh `./forge-server` start mints a local CA + leaf
    /// and serves HTTPS without any extra configuration. Set
    /// `[server.tls] enabled = false` explicitly to opt into plaintext.
    #[serde(default)]
    pub tls: TlsConfig,
}

/// TLS settings for the gRPC server.
///
/// Two modes:
/// - **Manual**: supply `cert_path` and `key_path` pointing at real PEM
///   files (e.g. from an ACME client). Leave `auto_generate = false`.
/// - **Auto-generate** (the default): set `auto_generate = true` and leave
///   `cert_path` and `key_path` unset. On first start, forge-server mints
///   a local CA and a leaf certificate covering `hostnames` + loopback,
///   writes them under `<base_path>/certs/`, and reuses them on every
///   subsequent start. Clients pin the CA via `forge login`'s trust-on-
///   first-use prompt.
///
/// To opt out of TLS entirely (loopback dev only), set
/// `[server.tls] enabled = false`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Master switch. Default true. Set false for plaintext h2c (only
    /// safe on loopback).
    #[serde(default = "default_tls_enabled")]
    pub enabled: bool,

    /// PEM-encoded certificate chain (leaf first). Defaults to
    /// `<base_path>/certs/server.crt`.
    #[serde(default)]
    pub cert_path: Option<PathBuf>,

    /// PEM-encoded private key matching the certificate. Defaults to
    /// `<base_path>/certs/server.key`.
    #[serde(default)]
    pub key_path: Option<PathBuf>,

    /// When true, generate a CA + leaf on first start if the files don't
    /// exist yet. **Default true.** When false, missing files are a
    /// startup error.
    #[serde(default = "default_tls_autogen")]
    pub auto_generate: bool,

    /// DNS names / IP addresses to encode into the leaf cert's
    /// `subjectAltName` extension. `localhost`, `127.0.0.1`, `::1`, and
    /// every non-loopback interface IP are always added implicitly.
    /// Ignored when `auto_generate` is false.
    #[serde(default)]
    pub hostnames: Vec<String>,
}

fn default_tls_enabled() -> bool {
    true
}
fn default_tls_autogen() -> bool {
    true
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: default_tls_enabled(),
            cert_path: None,
            key_path: None,
            auto_generate: default_tls_autogen(),
            hostnames: Vec::new(),
        }
    }
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

    /// When true, runs are executed only by registered agents; the
    /// embedded in-process runner stays idle. Flip this on once at least
    /// one `forge-agent` is registered — otherwise queued runs will
    /// pile up forever. Default false keeps single-host installs working
    /// out of the box.
    #[serde(default = "default_false")]
    pub use_agents: bool,
}

/// Artifact storage backend + retention policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactsSection {
    /// Storage backend. "fs" = local filesystem (default). "s3" = any
    /// S3-compatible endpoint (MinIO/R2/AWS). The s3 backend is compiled
    /// in behind the `s3` cargo feature; leaving it selected without the
    /// feature enabled is a startup error.
    #[serde(default = "default_artifacts_backend")]
    pub backend: String,

    /// Retention policy. Runs older than `max_days`, or runs outside the
    /// newest `max_runs_per_workflow` per workflow, are eligible for
    /// pruning. Release-pinned artifacts are always skipped.
    #[serde(default)]
    pub retention: ArtifactsRetention,

    /// S3 backend options (used only when `backend = "s3"`).
    #[serde(default)]
    pub s3: ArtifactsS3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactsRetention {
    #[serde(default = "default_retention_days")]
    pub max_days: u32,
    #[serde(default = "default_retention_runs")]
    pub max_runs_per_workflow: u32,
    /// Soft cap on total artifact bytes per repo. The prune job sorts
    /// eligible runs oldest-first and deletes until the repo is under the
    /// cap. 0 = unlimited.
    #[serde(default)]
    pub max_repo_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArtifactsS3 {
    /// Endpoint URL (e.g. `https://minio.example.com:9000` or
    /// `https://s3.amazonaws.com`). Leave empty for AWS-default.
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub bucket: String,
    /// When true, use path-style addressing (required for MinIO/R2/old
    /// S3-compatible endpoints). Default false = virtual-hosted style.
    #[serde(default)]
    pub path_style: bool,
    /// Optional key prefix under which all artifacts land, e.g. `"prod/"`.
    #[serde(default)]
    pub prefix: String,
}

fn default_artifacts_backend() -> String { "fs".into() }
fn default_retention_days() -> u32 { 30 }
fn default_retention_runs() -> u32 { 100 }

impl Default for ArtifactsRetention {
    fn default() -> Self {
        Self {
            max_days: default_retention_days(),
            max_runs_per_workflow: default_retention_runs(),
            max_repo_bytes: 0,
        }
    }
}

impl Default for ArtifactsSection {
    fn default() -> Self {
        Self {
            backend: default_artifacts_backend(),
            retention: ArtifactsRetention::default(),
            s3: ArtifactsS3::default(),
        }
    }
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
            use_agents: false,
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
            // TLS-on-by-default. The auto-gen path mints a CA + leaf
            // under <base_path>/certs/ on first start; no operator
            // intervention required.
            tls: TlsConfig::default(),
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
            artifacts: ArtifactsSection::default(),
            logging: LoggingSection::default(),
            limits: LimitsSection::default(),
            database: DatabaseSection::default(),
            objects: ObjectsSection::default(),
            metrics: MetricsSection::default(),
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

[metrics]
# Prometheus /metrics + /healthz + /readyz listener (plain HTTP — no
# TLS, scrape traffic stays on the internal network). Disable if you
# don't scrape; health probes (k8s liveness/readiness) work regardless.
enabled = true
listen  = "127.0.0.1:9877"
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

    /// Resolve the log directory. Returns `None` when file logging is
    /// disabled (empty `logging.dir`). Relative paths resolve against
    /// `storage.base_path` so an operator can set `dir = "logs"` and not
    /// have to think about absolute layout.
    pub fn resolved_log_dir(&self) -> Option<PathBuf> {
        if self.logging.dir.as_os_str().is_empty() {
            None
        } else if self.logging.dir.is_absolute() {
            Some(self.logging.dir.clone())
        } else {
            Some(self.storage.base_path.join(&self.logging.dir))
        }
    }
}
