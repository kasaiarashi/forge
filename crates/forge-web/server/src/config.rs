// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub web: WebConfig,
    pub server: ServerConfig,
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    /// Address to listen on, e.g. "0.0.0.0:3000"
    pub listen: String,
    /// Path to the static UI build output directory
    pub static_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// gRPC URL of the forge-server, e.g. "http://localhost:9876"
    pub grpc_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// bcrypt hash of the admin password.
    /// Generate with: forge-web hash-password <password>
    pub admin_password_hash: String,
    /// Secret used to sign session tokens
    pub session_secret: String,
    /// How long sessions last, in hours
    pub session_ttl_hours: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            web: WebConfig {
                listen: "0.0.0.0:3000".to_string(),
                static_dir: "./ui/dist".to_string(),
            },
            server: ServerConfig {
                grpc_url: "http://localhost:9876".to_string(),
            },
            auth: AuthConfig {
                admin_password_hash: String::new(),
                session_secret: "change-me-to-random-string".to_string(),
                session_ttl_hours: 24,
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
