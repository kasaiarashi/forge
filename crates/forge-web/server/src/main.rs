// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

mod api;
mod api_actions;
mod auth;
mod config;
mod grpc_client;

use std::path::PathBuf;
use std::sync::Arc;

use axum::middleware;
use axum::routing::{delete, get, post, put};
use axum::Router;
use clap::{Parser, Subcommand};
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

use crate::config::Config;
use crate::grpc_client::ForgeGrpcClient;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "forge-web", about = "Forge VCS Web UI server")]
struct Cli {
    /// Path to config file
    #[arg(long, default_value = "forge-web.toml")]
    config: PathBuf,

    /// Address to listen on (overrides config)
    #[arg(long)]
    listen: Option<String>,

    /// gRPC URL of forge-server (overrides config)
    #[arg(long)]
    grpc_url: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a default forge-web.toml config file
    Init,
    /// Hash a password for use in the config file
    HashPassword {
        /// The plaintext password to hash
        password: String,
    },
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub struct AppState {
    pub config: Config,
    /// Lazily-initialized gRPC client. Protected by RwLock so we can
    /// connect on first request (avoids startup failure if forge-server
    /// is not yet running).
    grpc: RwLock<Option<ForgeGrpcClient>>,
}

impl AppState {
    /// Get or create the gRPC client connection.
    pub async fn grpc_client(&self) -> anyhow::Result<ForgeGrpcClient> {
        // Fast path: already connected.
        {
            let guard = self.grpc.read().await;
            if let Some(ref client) = *guard {
                return Ok(client.clone());
            }
        }
        // Slow path: connect.
        let mut guard = self.grpc.write().await;
        // Double-check after acquiring write lock.
        if let Some(ref client) = *guard {
            return Ok(client.clone());
        }
        let client = ForgeGrpcClient::connect(&self.config.server.grpc_url).await?;
        *guard = Some(client.clone());
        Ok(client)
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Handle subcommands.
    match cli.command {
        Some(Commands::Init) => {
            let path = &cli.config;
            if path.exists() {
                anyhow::bail!("Config file already exists: {}", path.display());
            }
            Config::write_default(path)?;
            println!("Wrote default config to {}", path.display());
            println!("Edit the file and set auth.admin_password_hash.");
            println!("Generate a hash with: forge-web hash-password <password>");
            return Ok(());
        }
        Some(Commands::HashPassword { password }) => {
            let hash = bcrypt::hash(&password, bcrypt::DEFAULT_COST)?;
            println!("{hash}");
            return Ok(());
        }
        None => {}
    }

    // Load config.
    let mut cfg = if cli.config.exists() {
        Config::load(&cli.config)?
    } else {
        tracing::warn!(
            "Config file {} not found, using defaults",
            cli.config.display()
        );
        Config::default()
    };

    // CLI overrides.
    if let Some(listen) = cli.listen {
        cfg.web.listen = listen;
    }
    if let Some(grpc_url) = cli.grpc_url {
        cfg.server.grpc_url = grpc_url;
    }

    let listen_addr = cfg.web.listen.clone();
    let static_dir = PathBuf::from(&cfg.web.static_dir);

    let state = Arc::new(AppState {
        config: cfg,
        grpc: RwLock::new(None),
    });

    // ---- Build router ----

    // Public auth routes (no session required).
    let auth_routes = Router::new()
        .route("/login", post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/me", get(auth::me));

    // Public read-only API routes (no auth needed for browsing).
    let public_api = Router::new()
        .route("/repos", get(api::list_repos))
        .route("/repos/:repo/branches", get(api::list_branches))
        .route("/repos/:repo/commits/:branch", get(api::list_commits))
        .route("/repos/:repo/tree/:branch", get(api::get_tree))
        .route("/repos/:repo/blob/:branch", get(api::get_blob))
        .route("/repos/:repo/raw/:branch", get(api::get_raw))
        .route("/repos/:repo/commit/:hash", get(api::get_commit))
        .route("/repos/:repo/locks", get(api::list_locks))
        // Actions (public read)
        .route("/repos/:repo/workflows", get(api_actions::list_workflows))
        .route("/repos/:repo/runs", get(api_actions::list_runs))
        .route("/repos/:repo/runs/:run_id", get(api_actions::get_run))
        .route("/repos/:repo/runs/:run_id/artifacts", get(api_actions::list_artifacts))
        .route("/repos/:repo/releases", get(api_actions::list_releases))
        .route("/repos/:repo/releases/:release_id", get(api_actions::get_release));

    // Protected API routes (require auth for writes/admin).
    let protected_api = Router::new()
        .route("/repos", post(api::create_repo))
        .route("/repos/:repo", put(api::update_repo).delete(api::delete_repo))
        .route("/repos/:repo/locks/acquire", post(api::acquire_lock))
        .route("/repos/:repo/locks/:path", delete(api::release_lock))
        .route("/server/info", get(api::server_info))
        // Actions (protected writes)
        .route("/repos/:repo/workflows", post(api_actions::create_workflow))
        .route("/repos/:repo/workflows/:id", put(api_actions::update_workflow).delete(api_actions::delete_workflow))
        .route("/repos/:repo/workflows/:id/trigger", post(api_actions::trigger_workflow))
        .route("/repos/:repo/runs/:run_id/cancel", post(api_actions::cancel_run))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    let api_routes = Router::new()
        .nest("/auth", auth_routes)
        .merge(public_api)
        .merge(protected_api);

    // Static file serving -- serve index.html as fallback for SPA routing.
    let spa_service = if static_dir.exists() {
        let index_path = static_dir.join("index.html");
        ServeDir::new(&static_dir).fallback(ServeFile::new(index_path))
    } else {
        tracing::warn!(
            "Static dir {} does not exist; UI will not be served",
            static_dir.display()
        );
        // Fallback: serve from a non-existent dir (will 404).
        ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html")))
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .nest("/api", api_routes)
        .fallback_service(spa_service)
        .layer(cors)
        .with_state(state);

    // ---- Start server ----
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    tracing::info!("forge-web listening on {listen_addr}");
    axum::serve(listener, app).await?;

    Ok(())
}
