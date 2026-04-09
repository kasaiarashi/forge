// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

mod auth;
mod cli_admin;
mod config;
mod services;
mod storage;
mod update;

use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tonic::transport::Server;
use tracing::info;

use config::ServerConfig;
use forge_proto::forge::auth_service_server::AuthServiceServer;
use forge_proto::forge::forge_service_server::ForgeServiceServer;
use services::auth_service::ForgeAuthService;
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
    /// Manage users
    User {
        #[command(subcommand)]
        action: UserAction,
    },
    /// Manage per-repository access control
    Repo {
        #[command(subcommand)]
        action: RepoAction,
    },
    /// Check for updates and self-update the server
    Update {
        /// Only check for updates without installing
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum UserAction {
    /// Create a new user (interactive password prompt unless --password is given)
    Add {
        username: String,
        /// Email address (prompted if omitted)
        #[arg(long)]
        email: Option<String>,
        /// Display name (defaults to username)
        #[arg(long)]
        display_name: Option<String>,
        /// Make this user a server admin
        #[arg(long)]
        admin: bool,
        /// Set the password directly without prompting (avoid in shared shells)
        #[arg(long)]
        password: Option<String>,
    },
    /// List all users
    List,
    /// Delete a user (cascades to their sessions, PATs, and ACL grants)
    Delete { username: String },
    /// Reset a user's password
    ResetPassword {
        username: String,
        /// Set the password directly without prompting (avoid in shared shells)
        #[arg(long)]
        password: Option<String>,
    },
}

#[derive(Subcommand)]
enum RepoAction {
    /// Grant a user a role on a repo (read | write | admin)
    Grant {
        repo: String,
        username: String,
        /// One of: read, write, admin
        role: String,
    },
    /// Revoke a user's role on a repo
    Revoke { repo: String, username: String },
    /// List the users that have an explicit grant on a repo
    ListMembers { repo: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Update { check }) => {
            update::run(check)?;
            return Ok(());
        }
        Some(Commands::Init) => {
            let path = std::path::Path::new(&cli.config);
            if path.exists() {
                eprintln!("Config file already exists: {}", path.display());
                eprintln!("Delete it first or use a different path with --config.");
                std::process::exit(1);
            }
            std::fs::write(path, ServerConfig::generate_default())?;
            println!("Generated default config: {}", path.display());
            println!("\nNext steps:");
            println!("  1. Create the first admin:  forge-server user add --admin <username>");
            println!("  2. Start the server:        forge-server serve");
            return Ok(());
        }
        Some(Commands::User { ref action }) => {
            let config = load_config_for_admin(&cli)?;
            match action {
                UserAction::Add {
                    username,
                    email,
                    display_name,
                    admin,
                    password,
                } => cli_admin::user_add(
                    &config,
                    username,
                    email.as_deref(),
                    display_name.as_deref(),
                    *admin,
                    password.as_deref(),
                )?,
                UserAction::List => cli_admin::user_list(&config)?,
                UserAction::Delete { username } => cli_admin::user_delete(&config, username)?,
                UserAction::ResetPassword { username, password } => {
                    cli_admin::user_reset_password(&config, username, password.as_deref())?
                }
            }
            return Ok(());
        }
        Some(Commands::Repo { ref action }) => {
            let config = load_config_for_admin(&cli)?;
            match action {
                RepoAction::Grant {
                    repo,
                    username,
                    role,
                } => cli_admin::repo_grant(&config, repo, username, role)?,
                RepoAction::Revoke { repo, username } => {
                    cli_admin::repo_revoke(&config, repo, username)?
                }
                RepoAction::ListMembers { repo } => cli_admin::repo_list_members(&config, repo)?,
            }
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

    // Build the shared user store first so the gRPC service and the auth
    // interceptor / AuthService all share one Arc<dyn UserStore>.
    let user_store: Arc<dyn auth::UserStore> =
        Arc::new(auth::SqliteUserStore::new(Arc::clone(&db)));

    let service = ForgeGrpcService {
        fs: Arc::clone(&fs),
        db: Arc::clone(&db),
        start_time: std::time::Instant::now(),
        workflow_engine,
        user_store: Arc::clone(&user_store),
    };

    let addr = config.server.listen.parse()?;
    info!("Forge server listening on {}", addr);
    info!("Storage: {}", base.display());
    info!("Database: {}", db_path.display());

    let max_msg = config.server.max_message_size as usize;

    // The interceptor reuses the same store the gRPC service holds.
    let interceptor = auth::interceptor::make_interceptor(Arc::clone(&user_store));

    let forge_svc = ForgeServiceServer::new(service)
        .max_decoding_message_size(max_msg)
        .max_encoding_message_size(max_msg);
    let auth_svc = AuthServiceServer::new(ForgeAuthService {
        store: Arc::clone(&user_store),
    });

    Server::builder()
        .add_service(tonic::service::interceptor::InterceptedService::new(
            forge_svc,
            interceptor.clone(),
        ))
        .add_service(tonic::service::interceptor::InterceptedService::new(
            auth_svc,
            interceptor,
        ))
        .serve(addr)
        .await?;

    Ok(())
}

/// Load the server config the same way `serve` does, applying any global
/// `--storage` override. Used by the `user` and `repo` admin subcommands so
/// they hit the same database the running server would.
fn load_config_for_admin(cli: &Cli) -> Result<ServerConfig> {
    let config_path = std::path::Path::new(&cli.config);
    let mut config = if config_path.exists() {
        ServerConfig::load(config_path)?
    } else {
        ServerConfig::default()
    };
    if let Some(ref storage) = cli.storage {
        config.storage.base_path = storage.into();
    }
    Ok(config)
}
