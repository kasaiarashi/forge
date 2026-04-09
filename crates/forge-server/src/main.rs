// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

mod auth;
mod cli_admin;
mod config;
#[cfg(windows)]
mod service;
mod services;
mod storage;
mod tls_autogen;
mod update;

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tracing::{info, warn};

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

    /// Internal: hand off to the Windows Service Control Manager instead
    /// of running interactively. The installer-registered service has
    /// this flag baked into the binPath; users should never set it by
    /// hand. Hidden from `--help` to keep the surface area sane.
    #[arg(long, hide = true, global = true)]
    as_service: bool,

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
    /// Manage the Windows service (Windows only).
    #[cfg(windows)]
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[cfg(windows)]
#[derive(Subcommand)]
enum ServiceAction {
    /// Register forge-server with the Windows Service Control Manager
    /// and configure it to start automatically on boot.
    Install,
    /// Stop and remove the forge-server Windows service.
    Uninstall,
    /// Start the installed service.
    Start,
    /// Stop the running service.
    Stop,
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

// main is intentionally synchronous. The serve path builds its own Tokio
// runtime via [`run_serve`]; the Windows service path builds a separate
// runtime inside `service::run_under_scm`. Nesting `#[tokio::main]` would
// prevent the SCM dispatch from spinning up its own runtime cleanly.
fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    // Select a rustls crypto provider up-front. Both aws-lc-rs (via tonic's
    // tls feature) and ring (via axum-server's tls-rustls feature, if it
    // ever leaks in via a workspace dep) can end up in the build; when
    // that happens rustls refuses to pick one on its own and panics the
    // first time TLS is used. Install the default explicitly so that
    // enabling [server.tls] later does not blow up at handshake time.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

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
        #[cfg(windows)]
        Some(Commands::Service { ref action }) => {
            handle_service_command(action)?;
            return Ok(());
        }
        _ => {}
    }

    // Default action: serve. Load + apply CLI overrides, then dispatch
    // either to the SCM (if `--as-service`) or to a one-shot interactive
    // serve loop with Ctrl-C shutdown.
    let config = load_serve_config(&cli)?;

    #[cfg(windows)]
    {
        if cli.as_service {
            // The SCM launches every service process with
            // `cwd = C:\Windows\System32`, not the binary's directory.
            // That breaks any relative path in the config
            // (`static_dir = "./ui"`, `base_path = "./forge-data"`, the
            // TLS `cert_path`, etc.) because they end up resolving
            // against System32 where nothing lives. Pin cwd to the
            // binary's parent directory so the service mode matches the
            // interactive "cd to install dir and run the exe" case.
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    if let Err(e) = std::env::set_current_dir(dir) {
                        warn!(
                            error = %e,
                            "failed to set cwd to {} before service dispatch",
                            dir.display()
                        );
                    } else {
                        info!(
                            "service mode: cwd pinned to {}",
                            dir.display()
                        );
                    }
                }
            }

            // Hand control to the SCM. The dispatcher blocks until the
            // service stops. If we're not actually running under the SCM
            // (someone typed `--as-service` by hand), the dispatcher
            // returns ERROR_FAILED_SERVICE_CONTROLLER_CONNECT which we
            // surface as a clear error.
            return service::run_under_scm(service::ServicePayload { config });
        }
    }

    // Interactive run: build a runtime here, plumb Ctrl-C as the shutdown
    // signal so a regular console user can stop the server with ^C and we
    // still get a graceful tonic shutdown.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(serve_inner(config, async {
        let _ = tokio::signal::ctrl_c().await;
        info!("Ctrl-C received, shutting down");
    }))
}

/// Load the TOML config + apply `--listen` / `--storage` overrides. Used
/// by both the interactive and service-mode serve paths so the SCM-driven
/// startup picks up exactly the same config layering as a `serve` from a
/// console.
fn load_serve_config(cli: &Cli) -> Result<ServerConfig> {
    let config_path = std::path::Path::new(&cli.config);
    if !config_path.exists() {
        std::fs::write(config_path, ServerConfig::generate_default())?;
        info!("Created default config: {}", config_path.display());
    }
    let mut config = ServerConfig::load(config_path)?;
    if let Some(ref listen) = cli.listen {
        config.server.listen = listen.clone();
    }
    if let Some(ref storage) = cli.storage {
        config.storage.base_path = storage.into();
    }
    Ok(config)
}

/// Run the gRPC server until `shutdown` resolves. Extracted from the
/// inline body of `main` so the Windows service path
/// (`service::run_under_scm` -> `service::run_service`) can call it with
/// an SCM-driven shutdown future, while the console path passes
/// `ctrl_c().await`.
pub(crate) async fn serve_inner(
    mut config: ServerConfig,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    // Take ownership of base_path early so we can rebind it after moving
    // sections of `config` into the gRPC service.
    let base = config.storage.base_path.clone();
    std::fs::create_dir_all(base.join("repos"))?;

    let db_path = config.resolved_db_path();
    let db = Arc::new(MetadataDb::open(&db_path)?);

    // Bootstrap token: generated on first start (no users yet), written to
    // `<base_path>/.bootstrap_token`, and required on the BootstrapAdmin RPC.
    // Once the first admin is created we delete the file and stop enforcing.
    let bootstrap_token_path = base.join(".bootstrap_token");
    let bootstrap_token = ensure_bootstrap_token(Arc::clone(&db), &bootstrap_token_path)?;

    let repo_overrides: std::collections::HashMap<String, std::path::PathBuf> = config
        .repos
        .iter()
        .filter_map(|(name, rc)| rc.path.as_ref().map(|p| (name.clone(), p.clone())))
        .collect();
    let fs = Arc::new(FsStorage::new(base.join("repos"), repo_overrides));

    // Workflow engine is opt-in. See [actions] in forge-server.toml; the
    // post-audit default is OFF because steps run shell commands as the
    // forge-server process user.
    let workflow_engine = if config.actions.enabled {
        warn!(
            "*** Actions engine ENABLED — workflow steps will execute as shell \
             commands on this host. Ensure forge-server runs under an isolated \
             account. See docs/actions-security.md for the full threat model."
        );
        let tx = services::actions::engine::start(&config, Arc::clone(&db), Arc::clone(&fs));
        info!("Actions engine started (executor: {})", config.actions.executor);
        Some(tx)
    } else {
        None
    };

    let user_store: Arc<dyn auth::UserStore> =
        Arc::new(auth::SqliteUserStore::new(Arc::clone(&db)));

    let grpc_service = ForgeGrpcService {
        fs: Arc::clone(&fs),
        db: Arc::clone(&db),
        start_time: std::time::Instant::now(),
        workflow_engine,
        user_store: Arc::clone(&user_store),
    };

    let addr: std::net::SocketAddr = config.server.listen.parse()?;
    let tls_enabled = config.server.tls.enabled;
    let scheme = if tls_enabled { "https" } else { "http" };
    info!("Forge server listening on {scheme}://{}", addr);
    info!("Storage: {}", base.display());
    info!("Database: {}", db_path.display());

    if !tls_enabled && !addr.ip().is_loopback() {
        warn!(
            "forge-server is listening on {addr} WITHOUT TLS. Passwords, \
             PATs, and assets will traverse the network in plaintext. \
             Set [server.tls] enabled = true (the default) or bind to 127.0.0.1."
        );
    }

    let max_msg = config.server.max_message_size as usize;
    let interceptor = auth::interceptor::make_interceptor(Arc::clone(&user_store));

    let forge_svc = ForgeServiceServer::new(grpc_service)
        .max_decoding_message_size(max_msg)
        .max_encoding_message_size(max_msg);
    let auth_svc = AuthServiceServer::new(ForgeAuthService {
        store: Arc::clone(&user_store),
        bootstrap_token: bootstrap_token.clone(),
        bootstrap_token_path: bootstrap_token_path.clone(),
    });

    let mut builder = Server::builder();
    if config.server.tls.enabled {
        let tls = std::mem::take(&mut config.server.tls);
        let paths = resolve_tls_paths(&tls, &base);

        if tls.auto_generate {
            let mut sans = tls.hostnames.clone();
            let listen_ip = addr.ip();
            if listen_ip.is_unspecified() {
                for local in local_non_loopback_ips() {
                    let s = local.to_string();
                    if !sans.iter().any(|h| h == &s) {
                        sans.push(s);
                    }
                }
            } else {
                let host = listen_ip.to_string();
                if !sans.iter().any(|h| h == &host) {
                    sans.push(host);
                }
            }
            tls_autogen::ensure(&paths, &sans)
                .context("auto-generating TLS certificates")?;
        }

        let cert_pem = std::fs::read(&paths.leaf_cert)
            .with_context(|| format!("failed to read TLS cert {}", paths.leaf_cert.display()))?;
        let key_pem = std::fs::read(&paths.leaf_key)
            .with_context(|| format!("failed to read TLS key {}", paths.leaf_key.display()))?;
        let identity = Identity::from_pem(cert_pem, key_pem);
        builder = builder
            .tls_config(ServerTlsConfig::new().identity(identity))
            .context("tls_config failed")?;
        info!("TLS enabled: cert={}", paths.leaf_cert.display());

        if paths.ca_cert.exists() {
            if let Some(fp) = tls_autogen::cert_fingerprint(&paths.ca_cert) {
                warn!(
                    "\n*** TLS CA fingerprint (SHA-256):\n***   {fp}\n\
                     *** Clients should run `forge login --server https://<host>:{port}` \
                     and verify this fingerprint matches before accepting.\n\
                     *** CA cert file: {ca}",
                    port = addr.port(),
                    ca = paths.ca_cert.display()
                );
            }
        }
    }

    builder
        .add_service(tonic::service::interceptor::InterceptedService::new(
            forge_svc,
            interceptor.clone(),
        ))
        .add_service(tonic::service::interceptor::InterceptedService::new(
            auth_svc,
            interceptor,
        ))
        .serve_with_shutdown(addr, shutdown)
        .await?;

    info!("Forge server stopped cleanly");
    Ok(())
}

/// `forge-server service install/uninstall/start/stop` dispatcher.
#[cfg(windows)]
fn handle_service_command(action: &ServiceAction) -> Result<()> {
    match action {
        ServiceAction::Install => {
            // Resolve absolute paths so the binPath survives a working
            // directory change between install time and SCM start time.
            let bin = std::env::current_exe().context("locate forge-server.exe")?;
            let cfg_path = std::path::Path::new(&Cli::parse().config).to_path_buf();
            let cfg_abs = if cfg_path.is_absolute() {
                cfg_path
            } else {
                std::env::current_dir()?.join(cfg_path)
            };
            service::install(bin, cfg_abs)?;
            println!("Forge VCS Server installed as a Windows service.");
            println!("It will start automatically on boot. Run `forge-server service start` to start it now.");
            Ok(())
        }
        ServiceAction::Uninstall => {
            service::uninstall()?;
            println!("Forge VCS Server service removed.");
            Ok(())
        }
        ServiceAction::Start => {
            service::start()?;
            println!("Forge VCS Server service started.");
            Ok(())
        }
        ServiceAction::Stop => {
            service::stop()?;
            println!("Forge VCS Server service stopped.");
            Ok(())
        }
    }
}

/// Ensure a bootstrap token exists for a fresh install. When the users table
/// is empty and no token file has been created yet, generate a random token,
/// write it to `<base>/.bootstrap_token`, and log it loudly so the operator
/// can pair it with the web setup wizard.
///
/// Returns `None` when the server is already initialized (users exist). The
/// returned `Option<String>` is stashed on `ForgeAuthService` and compared
/// against `BootstrapAdminRequest.bootstrap_token`.
fn ensure_bootstrap_token(
    db: Arc<MetadataDb>,
    path: &std::path::Path,
) -> Result<Option<String>> {
    use auth::store::UserStore as _;
    let store = auth::SqliteUserStore::new(db);
    let user_count = store.count_users().context("counting users")?;
    if user_count > 0 {
        // Already initialized — make sure any leftover token file is gone.
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
        return Ok(None);
    }

    // Reuse an existing token if the server was restarted before the admin
    // finished the setup wizard.
    if let Ok(existing) = std::fs::read_to_string(path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            warn!(
                "*** BOOTSTRAP TOKEN (reusing existing from {:?}):\n    {}",
                path, trimmed
            );
            return Ok(Some(trimmed.to_string()));
        }
    }

    // Generate fresh 32-byte token as hex.
    let mut raw = [0u8; 32];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut raw);
    let token = hex::encode(raw);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(path, &token)
        .with_context(|| format!("failed to write {}", path.display()))?;

    warn!(
        "\n*** FIRST-RUN BOOTSTRAP TOKEN ***\n\
         *** Paste this into the web setup wizard to create the first admin.\n\
         *** Also saved to: {:?}\n\
         *** Token: {}\n",
        path, token
    );

    Ok(Some(token))
}

/// Enumerate every non-loopback, non-link-local IP the host has. Used
/// when the listen address is `0.0.0.0` / `::` so the auto-generated TLS
/// leaf can include every address a LAN client might reach us at, not
/// just the unspecified sentinel. Returns an empty vector on failure so
/// the startup path doesn't blow up — the operator can still use
/// `[server.tls].hostnames` as a manual override.
fn local_non_loopback_ips() -> Vec<std::net::IpAddr> {
    match if_addrs::get_if_addrs() {
        Ok(addrs) => addrs
            .into_iter()
            .filter_map(|iface| {
                let ip = iface.ip();
                if ip.is_loopback() {
                    return None;
                }
                // Skip IPv4 link-local (169.254.0.0/16) and IPv6 link-local
                // (fe80::/10). They're technically valid but almost never
                // what the operator means to expose.
                match ip {
                    std::net::IpAddr::V4(v4) if v4.is_link_local() => None,
                    std::net::IpAddr::V6(v6)
                        if (v6.segments()[0] & 0xffc0) == 0xfe80 =>
                    {
                        None
                    }
                    _ => Some(ip),
                }
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "failed to enumerate local interfaces");
            Vec::new()
        }
    }
}

/// Resolve the cert/key paths from `[server.tls]`, falling back to the
/// default layout under `<base_path>/certs/` when the operator left them
/// unset (the auto-generate happy path).
fn resolve_tls_paths(
    tls: &config::TlsConfig,
    base: &std::path::Path,
) -> tls_autogen::TlsPaths {
    let defaults = tls_autogen::TlsPaths::under(base);
    tls_autogen::TlsPaths {
        ca_cert: defaults.ca_cert.clone(),
        ca_key: defaults.ca_key.clone(),
        leaf_cert: tls.cert_path.clone().unwrap_or(defaults.leaf_cert),
        leaf_key: tls.key_path.clone().unwrap_or(defaults.leaf_key),
    }
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
