use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub storage_path: PathBuf,
    pub db_path: PathBuf,
    pub max_upload_size: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:9876".into(),
            storage_path: PathBuf::from("./forge-data/objects"),
            db_path: PathBuf::from("./forge-data/forge.db"),
            max_upload_size: 512 * 1024 * 1024, // 512 MiB
        }
    }
}
