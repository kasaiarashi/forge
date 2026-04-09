// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use serde::{Deserialize, Serialize};
use std::path::Path;

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
    /// Address to listen on, e.g. "0.0.0.0:3000"
    pub listen: String,
    /// Path to the static UI build output directory
    pub static_dir: String,
    /// Allowed CORS origins. Empty = mirror request origin (same-origin friendly).
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// gRPC URL of the forge-server, e.g. "http://localhost:9876"
    pub grpc_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            web: WebConfig {
                listen: "0.0.0.0:3000".to_string(),
                static_dir: "./ui/dist".to_string(),
                allowed_origins: vec![],
            },
            server: ServerConfig {
                grpc_url: "http://localhost:9876".to_string(),
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
