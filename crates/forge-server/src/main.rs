mod config;
mod services;
mod storage;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    // TODO: parse config, start gRPC server
    println!("Forge server — not yet implemented");
    Ok(())
}
