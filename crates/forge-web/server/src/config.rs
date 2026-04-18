// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

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
    #[serde(default)]
    pub logging: LoggingConfig,
}

/// Logging + audit sinks. Mirrors `forge-server`'s `LoggingSection`: file
/// sinks use daily rotation, `format` accepts `text` or `json`, and an
/// empty `dir` disables file logging (stdout-only behaviour).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_format")]
    pub format: String,
    #[serde(default)]
    pub dir: PathBuf,
    #[serde(default)]
    pub stdout: bool,
}

fn default_log_level() -> String {
    "info".into()
}
fn default_log_format() -> String {
    "text".into()
}

impl Default for LoggingConfig {
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

    /// Set the `Secure` attribute on session cookies. **Defaults to false**
    /// because the binary auto-escalates to true whenever it terminates TLS
    /// itself (see `secure_cookies = cfg.web.secure_cookies || tls_cfg.is_some()`
    /// in `main.rs`). Setting this to true while serving plaintext HTTP is
    /// a footgun: the browser stores the cookie but refuses to send it
    /// back, so the user can never stay logged in.
    ///
    /// Set to true explicitly only if you terminate TLS at a reverse proxy
    /// (so forge-web sees plaintext but the browser sees HTTPS).
    #[serde(default)]
    pub secure_cookies: bool,

    /// TLS configuration. **TLS is on by default.** Even if the operator's
    /// config file is missing the `[web.tls]` section entirely, the
    /// `TlsConfig::default()` produces `enabled = true` + `auto_generate
    /// = true`, so a fresh `./forge-web` start mints a local CA + leaf
    /// and serves HTTPS. Set `[web.tls] enabled = false` explicitly for
    /// plaintext (only safe behind a TLS-terminating reverse proxy or on
    /// loopback).
    #[serde(default)]
    pub tls: TlsConfig,

    /// Request-rate limits applied to `/api/auth/*`. Absent = defaults.
    #[serde(default)]
    pub rate_limit: RateLimitConfig,

    /// Plain HTTP port. When TLS is enabled, requests on this port are
    /// 308-redirected to the HTTPS URL on `https_port`. When TLS is off,
    /// this is the port the server listens on. Bound on the same interface
    /// as `listen`. Default: 80.
    #[serde(default = "default_http_port")]
    pub http_port: u16,

    /// HTTPS port. Only used when TLS is enabled. Default: 443.
    #[serde(default = "default_https_port")]
    pub https_port: u16,
}

/// TLS settings for the HTTPS listener.
///
/// Two modes:
/// - **Manual**: supply `cert_path` and `key_path` pointing at real PEM
///   files (e.g. a Let's Encrypt leaf). Set `auto_generate = false`.
/// - **Auto-generate** (the default): on first start, forge-web mints a
///   local CA and a leaf cert covering `hostnames` + loopback, writes
///   them under `./forge-web-certs/`, and reuses them on every restart.
///   Browsers will still prompt for "unknown certificate authority"
///   until the operator imports the CA into the OS trust store — there
///   is no way around that without ACME.
///
/// To opt out of TLS entirely, set `[web.tls] enabled = false`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Master switch. Default true.
    #[serde(default = "default_tls_enabled")]
    pub enabled: bool,
    /// PEM-encoded certificate chain (leaf first). Defaults to
    /// `./forge-web-certs/server.crt`.
    #[serde(default)]
    pub cert_path: Option<PathBuf>,
    /// PEM-encoded private key. Defaults to
    /// `./forge-web-certs/server.key`.
    #[serde(default)]
    pub key_path: Option<PathBuf>,
    /// When true (the default), generate a CA + leaf on first start if
    /// missing.
    #[serde(default = "default_tls_autogen")]
    pub auto_generate: bool,
    /// DNS names / IP addresses for the leaf cert's SAN. Loopback entries
    /// and every detected non-loopback interface IP are always added
    /// implicitly.
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
    "0.0.0.0".to_string()
}
fn default_http_port() -> u16 {
    80
}
fn default_https_port() -> u16 {
    443
}
fn default_static_dir() -> String {
    "./crates/forge-web/ui/dist".to_string()
}
fn default_grpc_url() -> String {
    "http://127.0.0.1:9876".to_string()
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
                secure_cookies: false,
                tls: TlsConfig::default(),
                rate_limit: RateLimitConfig::default(),
                http_port: default_http_port(),
                https_port: default_https_port(),
            },
            server: ServerConfig {
                grpc_url: default_grpc_url(),
                ca_cert_path: None,
            },
            logging: LoggingConfig::default(),
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
