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
    /// Allowed CORS origins. Empty = mirror request origin (same-origin friendly).
    #[serde(default)]
    pub allowed_origins: Vec<String>,
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
    /// Secret used to sign JWT tokens
    pub jwt_secret: String,
    /// How long tokens last, in hours
    pub token_ttl_hours: u64,
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
            auth: AuthConfig {
                admin_password_hash: String::new(),
                jwt_secret: "change-me-to-random-string".to_string(),
                token_ttl_hours: 24,
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
