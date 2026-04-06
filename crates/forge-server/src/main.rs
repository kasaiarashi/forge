// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

mod config;
mod services;
mod storage;

use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tonic::transport::Server;
use tracing::info;

use config::ServerConfig;
use forge_proto::forge::forge_service_server::ForgeServiceServer;
use services::grpc::ForgeGrpcService;
use storage::db::MetadataDb;
use storage::fs::FsStorage;

#[derive(Parser)]
#[command(name = "forge-server", about = "Forge VCS server", version)]
struct Cli {
    /// Path to config file (TOML)
    #[arg(short, long, default_value = "forge-server.toml", global = true)]
    config: String,

    /// Override listen address
    #[arg(short, long, global = true)]
    listen: Option<String>,

    /// Override storage base path
    #[arg(short, long, global = true)]
    storage: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a default config file
    Init,
    /// Start the server (default)
    Serve,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Init) => {
            let path = std::path::Path::new(&cli.config);
            if path.exists() {
                eprintln!("Config file already exists: {}", path.display());
                eprintln!("Delete it first or use a different path with --config.");
                std::process::exit(1);
            }
            std::fs::write(path, ServerConfig::generate_default())?;
            println!("Generated default config: {}", path.display());
            println!("\nEdit it to configure storage paths, then run:");
            println!("  forge-server serve");
            return Ok(());
        }
        _ => {}
    }

    // Load config file (uses defaults if file doesn't exist).
    let mut config = ServerConfig::load(std::path::Path::new(&cli.config))?;

    // CLI overrides.
    if let Some(listen) = cli.listen {
        config.server.listen = listen;
    }
    if let Some(storage) = cli.storage {
        config.storage.base_path = storage.into();
    }

    // Ensure base directories exist.
    let base = &config.storage.base_path;
    std::fs::create_dir_all(base.join("repos"))?;

    let db_path = config.resolved_db_path();
    let db = Arc::new(MetadataDb::open(&db_path)?);

    let fs = Arc::new(FsStorage::new(base.join("repos")));

    let service = ForgeGrpcService {
        fs: Arc::clone(&fs),
        db: Arc::clone(&db),
        start_time: std::time::Instant::now(),
    };

    let addr = config.server.listen.parse()?;
    info!("Forge server listening on {}", addr);
    info!("Storage: {}", base.display());
    info!("Database: {}", db_path.display());

    Server::builder()
        .add_service(ForgeServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
