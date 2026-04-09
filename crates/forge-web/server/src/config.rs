// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Web UI server configuration. Auth is no longer carried here — it lives
/// entirely on the forge-server side now (users + sessions + PATs in the
/// shared SQLite metadata DB). This server is just an HTTP shell that
/// translates browser cookies into gRPC bearer tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub web: WebConfig,
    pub server: ServerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    /// Address to listen on. Defaults to loopback so fresh installs are not
    /// immediately internet-exposed.
    #[serde(default = "default_listen")]
    pub listen: String,

    /// Path to the static UI build output directory.
    #[serde(default = "default_static_dir")]
    pub static_dir: String,

    /// Allowed CORS origins. Empty (the default) means *no* cross-origin
    /// requests are authorized — the SPA is served from the same origin as
    /// the API, so CORS is not needed. Never wildcard this list when
    /// `tls` is absent.
    #[serde(default)]
    pub allowed_origins: Vec<String>,

    /// Set the `Secure` attribute on session cookies. Default true. Flip to
    /// false only when developing against plaintext http://127.0.0.1 — any
    /// other deployment should keep this on.
    #[serde(default = "default_true")]
    pub secure_cookies: bool,

    /// Optional TLS configuration. When present, the web server terminates
    /// TLS itself using rustls; when absent, it serves plaintext HTTP (the
    /// only sensible scenario for that is loopback behind a separate TLS
    /// reverse proxy).
    #[serde(default)]
    pub tls: Option<TlsConfig>,

    /// Request-rate limits applied to `/api/auth/*`. Absent = defaults.
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
}

/// TLS settings for the HTTPS listener.
///
/// Two modes:
/// - **Manual**: supply `cert_path` and `key_path` pointing at real PEM
///   files (e.g. a Let's Encrypt leaf).
/// - **Auto-generate**: set `auto_generate = true` and leave the paths at
///   their defaults. On first start, forge-web mints a local CA and a leaf
///   cert covering `hostnames` + loopback, writes them under
///   `./forge-web-certs/`, and reuses them on every restart. Browsers will
///   still prompt for "unknown certificate authority" until the operator
///   imports the CA into the OS trust store — there is no way around that
///   without ACME.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// PEM-encoded certificate chain (leaf first). Defaults to
    /// `./forge-web-certs/server.crt`.
    #[serde(default)]
    pub cert_path: Option<PathBuf>,
    /// PEM-encoded private key. Defaults to
    /// `./forge-web-certs/server.key`.
    #[serde(default)]
    pub key_path: Option<PathBuf>,
    /// When true, generate a CA + leaf on first start if missing.
    #[serde(default)]
    pub auto_generate: bool,
    /// DNS names / IP addresses for the leaf cert's SAN. Loopback entries
    /// are always added implicitly.
    #[serde(default)]
    pub hostnames: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Sustained requests per second per client IP.
    #[serde(default = "default_rl_per_second")]
    pub per_second: u64,
    /// Burst budget before throttling kicks in.
    #[serde(default = "default_rl_burst")]
    pub burst: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            per_second: default_rl_per_second(),
            burst: default_rl_burst(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// gRPC URL of the forge-server, e.g. "https://forge.example.com:9876".
    #[serde(default = "default_grpc_url")]
    pub grpc_url: String,

    /// Optional CA bundle (PEM) to trust when connecting to a forge-server
    /// with a self-signed certificate. Required for self-signed TLS.
    #[serde(default)]
    pub ca_cert_path: Option<PathBuf>,
}

fn default_listen() -> String {
    "127.0.0.1:3000".to_string()
}
fn default_static_dir() -> String {
    "./crates/forge-web/ui/dist".to_string()
}
fn default_grpc_url() -> String {
    "http://127.0.0.1:9876".to_string()
}
fn default_true() -> bool {
    true
}
fn default_rl_per_second() -> u64 {
    1
}
fn default_rl_burst() -> u32 {
    5
}

impl Default for Config {
    fn default() -> Self {
        Self {
            web: WebConfig {
                listen: default_listen(),
                static_dir: default_static_dir(),
                allowed_origins: vec![],
                secure_cookies: true,
                tls: None,
                rate_limit: RateLimitConfig::default(),
            },
            server: ServerConfig {
                grpc_url: default_grpc_url(),
                ca_cert_path: None,
            },
        }
    }
}

impl Config {
    /// Load config from a TOML file, falling back to defaults for missing fields.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Write the default config to a TOML file.
    pub fn write_default(path: &Path) -> anyhow::Result<()> {
        let config = Config::default();
        let contents = toml::to_string_pretty(&config)?;
        std::fs::write(path, contents)?;
        Ok(())
    }
}
