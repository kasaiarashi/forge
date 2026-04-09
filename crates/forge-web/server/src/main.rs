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
use tower_http::cors::CorsLayer;
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
    /// Get or create the gRPC client connection. Idempotent and lazy.
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

    /// Build a fresh `AuthServiceClient` for one request, with the current
    /// task-local session token attached. Used by `auth.rs` handlers.
    pub async fn grpc_auth_client(
        &self,
    ) -> anyhow::Result<
        forge_proto::forge::auth_service_client::AuthServiceClient<
            tonic::service::interceptor::InterceptedService<
                tonic::transport::Channel,
                crate::grpc_client::BearerInterceptor,
            >,
        >,
    > {
        let client = self.grpc_client().await?;
        Ok(client.auth())
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
            println!();
            println!("Next steps:");
            println!(
                "  1. Make sure forge-server is running and create your first admin:"
            );
            println!("       forge-server user add --admin <username>");
            println!("     (or visit /setup in the browser after starting forge-web).");
            println!("  2. Start forge-web:");
            println!("       forge-web");
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
    let allowed_origins = cfg.web.allowed_origins.clone();

    let state = Arc::new(AppState {
        config: cfg,
        grpc: RwLock::new(None),
    });

    // ---- Build router ----

    // Auth routes. Login / logout / me / setup wizard are public; everything
    // else (token mint, session list, user admin, repo ACL admin) is gated by
    // the gRPC server's per-handler authz check, which reads the bearer token
    // we forward from the cookie.
    let auth_routes = Router::new()
        .route("/login", post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/initialized", get(auth::is_initialized))
        .route("/bootstrap", post(auth::bootstrap_admin))
        .route("/tokens", get(auth::list_tokens).post(auth::create_token))
        .route("/tokens/:id", delete(auth::delete_token))
        .route(
            "/sessions",
            get(auth::list_sessions),
        )
        .route("/sessions/:id", delete(auth::delete_session))
        .route(
            "/users",
            get(auth::list_users).post(auth::create_user),
        )
        .route("/users/:id", delete(auth::delete_user))
        .route(
            "/repos/:repo/members",
            get(auth::list_repo_members).post(auth::grant_repo_role),
        )
        .route("/repos/:repo/members/:user_id", delete(auth::revoke_repo_role));

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
        .route("/repos/:repo/stats/languages", get(api::language_stats))
        // Issues & Pull Requests (public read)
        .route("/repos/:repo/issues", get(api::list_issues))
        .route("/repos/:repo/issues/:id", get(api::get_issue))
        .route("/repos/:repo/pulls", get(api::list_pull_requests))
        .route("/repos/:repo/pulls/:id", get(api::get_pull_request))
        // Actions (public read)
        .route("/repos/:repo/workflows", get(api_actions::list_workflows))
        .route("/repos/:repo/runs", get(api_actions::list_runs))
        .route("/repos/:repo/runs/:run_id", get(api_actions::get_run))
        .route("/repos/:repo/runs/:run_id/artifacts", get(api_actions::list_artifacts))
        .route("/repos/:repo/releases", get(api_actions::list_releases))
        .route("/repos/:repo/releases/:release_id", get(api_actions::get_release));

    // Mutating routes. The gRPC server's per-handler authz now enforces
    // authentication and per-repo roles, so the web server doesn't need a
    // separate require_auth middleware. The session_token_layer (installed
    // below at the top-level router) takes care of forwarding the cookie
    // through to the upstream gRPC call.
    let protected_api = Router::new()
        .route("/repos", post(api::create_repo))
        .route("/repos/:repo", put(api::update_repo).delete(api::delete_repo))
        // Issues & Pull Requests (writes)
        .route("/repos/:repo/issues", post(api::create_issue))
        .route("/repos/:repo/issues/:id", put(api::update_issue))
        .route("/repos/:repo/pulls", post(api::create_pull_request))
        .route("/repos/:repo/pulls/:id", put(api::update_pull_request))
        .route("/repos/:repo/pulls/:id/merge", post(api::merge_pull_request))
        .route("/repos/:repo/locks/acquire", post(api::acquire_lock))
        .route("/repos/:repo/locks/:path", delete(api::release_lock))
        .route("/server/info", get(api::server_info))
        // Actions (writes)
        .route("/repos/:repo/workflows", post(api_actions::create_workflow))
        .route("/repos/:repo/workflows/:id", put(api_actions::update_workflow).delete(api_actions::delete_workflow))
        .route("/repos/:repo/workflows/:id/trigger", post(api_actions::trigger_workflow))
        .route("/repos/:repo/runs/:run_id/cancel", post(api_actions::cancel_run));

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

    let cors = {
        let base = CorsLayer::new()
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
                axum::http::header::COOKIE,
            ])
            .allow_credentials(true);

        if allowed_origins.is_empty() {
            base.allow_origin(tower_http::cors::AllowOrigin::mirror_request())
        } else {
            let origins: Vec<axum::http::HeaderValue> = allowed_origins
                .iter()
                .filter_map(|o| o.parse().ok())
                .collect();
            base.allow_origin(origins)
        }
    };

    let app = Router::new()
        .nest("/api", api_routes)
        .fallback_service(spa_service)
        // Run every request inside the session-token task-local scope so the
        // gRPC client can read the cookie's session token without rewriting
        // every handler signature. Layer order matters: cors must wrap the
        // session layer because the cors preflight responses don't need a
        // token. The session layer wraps with_state so handlers see the
        // task-local already populated.
        .layer(middleware::from_fn(auth::session_token_layer))
        .layer(cors)
        .with_state(state);

    // ---- Start server ----
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    tracing::info!("forge-web listening on {listen_addr}");
    axum::serve(listener, app).await?;

    Ok(())
}
