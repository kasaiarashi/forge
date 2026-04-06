// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

mod config;
mod services;
mod storage;

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tonic::transport::Server;
use tracing::info;

use forge_proto::forge::forge_service_server::ForgeServiceServer;
use services::grpc::ForgeGrpcService;
use storage::db::MetadataDb;
use storage::fs::FsStorage;

#[derive(Parser)]
#[command(name = "forge-server", about = "Forge VCS server", version)]
struct Cli {
    /// Address to listen on
    #[arg(short, long, default_value = "0.0.0.0:9876")]
    listen: String,

    /// Directory for object storage
    #[arg(short, long, default_value = "./forge-data/objects")]
    storage: String,

    /// Path to SQLite database
    #[arg(short, long, default_value = "./forge-data/forge.db")]
    database: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let addr = cli.listen.parse()?;
    let fs = Arc::new(FsStorage::new(cli.storage.into()));
    let db = Arc::new(MetadataDb::open(std::path::Path::new(&cli.database))?);

    let service = ForgeGrpcService {
        fs: Arc::clone(&fs),
        db: Arc::clone(&db),
    };

    info!("Forge server listening on {}", addr);

    Server::builder()
        .add_service(ForgeServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
