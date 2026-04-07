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

    // Load config file; auto-create default if it doesn't exist.
    let config_path = std::path::Path::new(&cli.config);
    if !config_path.exists() {
        std::fs::write(config_path, ServerConfig::generate_default())?;
        info!("Created default config: {}", config_path.display());
    }
    let mut config = ServerConfig::load(config_path)?;

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

    let repo_overrides: std::collections::HashMap<String, std::path::PathBuf> = config
        .repos
        .iter()
        .filter_map(|(name, rc)| rc.path.as_ref().map(|p| (name.clone(), p.clone())))
        .collect();
    let fs = Arc::new(FsStorage::new(base.join("repos"), repo_overrides));

    // Start workflow engine if actions are enabled.
    let workflow_engine = if config.actions.enabled {
        let tx = services::actions::engine::start(&config, Arc::clone(&db), Arc::clone(&fs));
        info!("Actions engine started (executor: {})", config.actions.executor);
        Some(tx)
    } else {
        None
    };

    let service = ForgeGrpcService {
        fs: Arc::clone(&fs),
        db: Arc::clone(&db),
        start_time: std::time::Instant::now(),
        workflow_engine,
    };

    let addr = config.server.listen.parse()?;
    info!("Forge server listening on {}", addr);
    info!("Storage: {}", base.display());
    info!("Database: {}", db_path.display());

    let max_msg = config.server.max_message_size as usize;

    let auth_interceptor = services::auth::make_auth_interceptor(
        config.auth.enabled,
        config.auth.tokens.clone(),
    );

    let svc = ForgeServiceServer::new(service)
        .max_decoding_message_size(max_msg)
        .max_encoding_message_size(max_msg);

    Server::builder()
        .add_service(tonic::service::interceptor::InterceptedService::new(svc, auth_interceptor))
        .serve(addr)
        .await?;

    Ok(())
}
