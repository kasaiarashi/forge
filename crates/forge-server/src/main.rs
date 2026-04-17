// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

mod auth;
#[cfg(windows)]
mod cert_install;
mod cli_admin;
mod config;
mod observability;
#[cfg(windows)]
mod service;
mod services;
mod storage;
mod tls_autogen;
mod update;
#[cfg(target_os = "linux")]
mod uninstall;

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tracing::{info, warn};

use config::ServerConfig;
use forge_proto::forge::agent_service_server::AgentServiceServer;
use forge_proto::forge::auth_service_server::AuthServiceServer;
use forge_proto::forge::forge_service_server::ForgeServiceServer;
use services::auth_service::ForgeAuthService;
use services::grpc::ForgeGrpcService;
use storage::db::MetadataDb;
use storage::fs::FsStorage;

#[derive(Parser)]
#[command(
    name = "forge-server",
    about = "Forge VCS server",
    version,
    long_version = concat!(
        env!("CARGO_PKG_VERSION"), "\n",
        "Copyright (c) 2026 Krishna Teja Mekala \u{2014} https://github.com/kasaiarashi/forge\n",
        "Licensed under BSL 1.1",
    ),
)]
struct Cli {
    /// Path to config file (TOML). Defaults to `forge-server.toml` in the
    /// current directory; if that file is absent on Linux, falls back to
    /// `/etc/forge/forge-server.toml` (the installer's canonical location)
    /// so admin CLI commands work from any cwd without `--config`.
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
    /// Print version info (same format as `forge --version`)
    Info,
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
    /// Manage CI agents (add/list/remove)
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Check for updates and self-update the server
    Update {
        /// Only check for updates without installing
        #[arg(long)]
        check: bool,

        /// Force re-download even if already on the latest version
        #[arg(short, long)]
        force: bool,

        /// Install a specific version tag (e.g. `0.1.0` or `v0.1.0`).
        /// Defaults to the latest release.
        #[arg(long, value_name = "TAG")]
        version: Option<String>,
    },
    /// Uninstall forge-server from this Linux host (binaries, config,
    /// systemd units, profile snippet). Use --purge to also remove data.
    #[cfg(target_os = "linux")]
    Uninstall {
        /// Also delete the data directory (DB, objects, certs). Irreversible.
        #[arg(long)]
        purge: bool,

        /// Skip the interactive confirmation prompt.
        #[arg(short, long)]
        yes: bool,
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

#[derive(Subcommand)]
enum AgentAction {
    /// Provision a new agent and print its token (show once).
    Add {
        name: String,
        /// Labels the agent will advertise, e.g. `--labels os:windows ue:5.7`.
        #[arg(long, num_args = 0..)]
        labels: Vec<String>,
    },
    /// List registered agents.
    List,
    /// Remove an agent (breaks its token immediately).
    Remove { name: String },
}

// main is intentionally synchronous. The serve path builds its own Tokio
// runtime via [`run_serve`]; the Windows service path builds a separate
// runtime inside `service::run_under_scm`. Nesting `#[tokio::main]` would
// prevent the SCM dispatch from spinning up its own runtime cleanly.
fn main() -> Result<()> {
    // tracing is initialised lazily: admin subcommands don't use it and
    // the `serve` path calls [`observability::init`] after the config is
    // in hand so file/audit sinks can be wired up correctly. A bare early
    // `fmt::init` here would grab the global subscriber slot and prevent
    // the richer init from running.

    // Select a rustls crypto provider up-front. Both aws-lc-rs (via tonic's
    // tls feature) and ring (via axum-server's tls-rustls feature, if it
    // ever leaks in via a workspace dep) can end up in the build; when
    // that happens rustls refuses to pick one on its own and panics the
    // first time TLS is used. Install the default explicitly so that
    // enabling [server.tls] later does not blow up at handshake time.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // `mut` is consumed by the Linux config-fallback block below; keep
    // the binding mutable unconditionally so the attribute doesn't flip
    // with cfg, and silence the spurious warning on non-Linux targets.
    #[allow(unused_mut)]
    let mut cli = Cli::parse();

    // Fall back to the system-wide config location when the user didn't
    // pass --config and the cwd-relative default isn't present. Without
    // this, admin CLI commands (`forge-server user list`, etc.) invoked
    // from a user's shell fail with SQLITE_READONLY_DIRECTORY because
    // the auto-generated default config points `base_path` at
    // `./forge-data`, which resolves under the binary's dir (/usr/local/bin,
    // not writable). On Linux we resolve to the installer's canonical
    // `/etc/forge/forge-server.toml` when it exists.
    #[cfg(target_os = "linux")]
    {
        if cli.config == "forge-server.toml"
            && !std::path::Path::new(&cli.config).exists()
        {
            let system = "/etc/forge/forge-server.toml";
            if std::path::Path::new(system).exists() {
                cli.config = system.into();
            }
        }
    }

    // Resolve `--config` to an absolute path *before* we change the cwd.
    // The chdir-to-binary-dir below would otherwise reinterpret a relative
    // `--config forge-server.toml` as living next to the binary, silently
    // ignoring the file the user pointed at. Canonicalize when the file
    // exists; fall back to plain cwd-join when it doesn't (so `init` still
    // creates the file at the path the user typed).
    {
        let p = std::path::Path::new(&cli.config);
        if !p.is_absolute() {
            let abs = std::fs::canonicalize(p)
                .ok()
                .or_else(|| std::env::current_dir().ok().map(|cwd| cwd.join(p)));
            if let Some(abs) = abs {
                cli.config = abs.to_string_lossy().into_owned();
            }
        }
    }

    // Always run from the binary's directory so config, data paths, and
    // certs resolve relative to where the binary lives — not wherever
    // the user happened to launch it from.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let _ = std::env::set_current_dir(dir);
        }
    }

    match cli.command {
        Some(Commands::Info) => {
            println!(
                "forge-server {}\nCopyright (c) 2026 Krishna Teja Mekala \u{2014} https://github.com/kasaiarashi/forge\nLicensed under BSL 1.1",
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        }
        Some(Commands::Update { check, force, version }) => {
            update::run(check, force, version)?;
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        Some(Commands::Uninstall { purge, yes }) => {
            uninstall::run(purge, yes)?;
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
        Some(Commands::Agent { ref action }) => {
            let config = load_config_for_admin(&cli)?;
            match action {
                AgentAction::Add { name, labels } => {
                    cli_admin::agent_add(&config, name, labels)?
                }
                AgentAction::List => cli_admin::agent_list(&config)?,
                AgentAction::Remove { name } => cli_admin::agent_remove(&config, name)?,
            }
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
    resolve_base_path_relative_to_config(&mut config, config_path);
    if let Some(ref listen) = cli.listen {
        config.server.listen = listen.clone();
    }
    if let Some(ref storage) = cli.storage {
        config.storage.base_path = storage.into();
    }
    Ok(config)
}

/// If `config.storage.base_path` is a relative path, anchor it to the
/// directory the config file lives in. Without this, `base_path = "./forge-data"`
/// (the default) resolves against whatever cwd happened to launch the
/// server — and a restart from a different shell silently picks a brand
/// new directory, mints a fresh self-signed CA, and breaks every client
/// that had already pinned the old fingerprint.
///
/// We canonicalize the config path so the parent is always absolute even
/// when the user passed `--config forge-server.toml` (parent would be
/// empty string otherwise). Falls back to cwd only if canonicalization
/// fails — which would mean the file doesn't exist, in which case we're
/// already in a degraded state and the existing relative behavior is no
/// worse than before.
fn resolve_base_path_relative_to_config(config: &mut ServerConfig, config_path: &std::path::Path) {
    if config.storage.base_path.is_absolute() {
        return;
    }
    let config_dir = std::fs::canonicalize(config_path)
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    config.storage.base_path = config_dir.join(&config.storage.base_path);
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

    // Wire up logging + audit sinks now that we have a config and a base
    // path to resolve the log dir against. Guards are held until the end
    // of `serve_inner`; dropping them at the wrong moment loses the
    // final flush from the non-blocking appender.
    let _log_guards = observability::init(
        &config.logging,
        config.resolved_log_dir().as_deref(),
    );

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

    let user_store: Arc<dyn auth::UserStore> =
        Arc::new(auth::SqliteUserStore::new(Arc::clone(&db)));

    // Secrets: load/create master key under <base>/secrets/master.key, then
    // wrap the DB in the AES-GCM SQLite backend. Swap to a KMS-backed
    // SecretBackend here later without touching call sites.
    let master_key = services::secrets::master_key::load_or_create(&base)
        .context("load or create secrets master key")?;
    let secrets: Arc<dyn services::secrets::SecretBackend> = Arc::new(
        services::secrets::sqlite::SqliteSecretBackend::new(Arc::clone(&db), &master_key),
    );

    // Artifact store: FS (default) or S3 (feature-gated). Matches
    // `[artifacts] backend = ...` in the config. Selecting `"s3"` without
    // the `s3` cargo feature is a hard error at startup rather than a
    // silent downgrade to FS.
    let artifacts_root = config.resolved_artifacts_path();
    let artifacts: Arc<dyn services::artifacts::ArtifactStore> =
        match config.artifacts.backend.as_str() {
            "fs" => Arc::new(services::artifacts::fs::FsArtifactStore::new(
                artifacts_root.clone(),
            )),
            "s3" => {
                // Trait-compatible stub: constructs fine, validates config,
                // but every put/get returns a clear "not implemented" error.
                // Keeps the wiring honest so a real S3 client is a drop-in.
                warn!(
                    "artifacts backend = \"s3\" is a stub in this build; \
                     uploads will fail. Use backend = \"fs\" for production."
                );
                Arc::new(services::artifacts::s3::S3ArtifactStore::new(
                    config.artifacts.s3.clone(),
                )?)
            }
            other => anyhow::bail!("unknown artifact backend: {}", other),
        };
    // Retention sweeper. No-op when the actions engine is off and no runs
    // are ever produced, but safe to start unconditionally.
    services::artifacts::retention::spawn(
        Arc::clone(&db),
        Arc::clone(&artifacts),
        config.artifacts.retention.clone(),
    );

    // Agent heartbeat sweeper. Requeues runs whose owning agent has gone
    // silent so a crashed worker can't hold a claim forever.
    services::agent_sweeper::spawn(Arc::clone(&db));
    services::session_sweeper::spawn(Arc::clone(&db), Arc::clone(&fs));

    // Live step-log broadcast hub. Engine + (future) agents publish;
    // StreamStepLogs readers subscribe.
    let log_hub = Arc::new(services::logs::LogHub::new());

    // Composite actions registry: copy the bundled `actions/` tree (shipped
    // next to the server binary or resolved via the repo's actions dir)
    // into `<base>/actions/` on every start. Operator overrides dropped
    // directly in `<base>/actions/` survive — we only refresh files that
    // differ from the bundled copy, never delete strays.
    let actions_root = base.join("actions");
    if let Err(e) = sync_bundled_actions(&actions_root) {
        warn!(error = %e, "failed to sync bundled actions (server will still start)");
    }

    // Workflow engine is opt-in. See [actions] in forge-server.toml; the
    // post-audit default is OFF because steps run shell commands as the
    // forge-server process user. When `[actions] use_agents = true`, skip
    // the in-process runner entirely so only external agents pick up runs.
    let workflow_engine = if config.actions.enabled && !config.actions.use_agents {
        warn!(
            "*** Actions engine ENABLED — workflow steps will execute as shell \
             commands on this host. Ensure forge-server runs under an isolated \
             account. See docs/actions-security.md for the full threat model."
        );
        let tx = services::actions::engine::start(
            &config,
            Arc::clone(&db),
            Arc::clone(&fs),
            Arc::clone(&secrets),
            Arc::clone(&log_hub),
        );
        info!("Actions engine started (executor: {})", config.actions.executor);
        Some(tx)
    } else {
        None
    };

    let grpc_service = ForgeGrpcService {
        fs: Arc::clone(&fs),
        db: Arc::clone(&db),
        start_time: std::time::Instant::now(),
        workflow_engine,
        user_store: Arc::clone(&user_store),
        secrets: Arc::clone(&secrets),
        artifacts: Arc::clone(&artifacts),
        artifact_signer_key: master_key,
        log_hub: Arc::clone(&log_hub),
        limits: config.limits.clone(),
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
    let agent_svc = AgentServiceServer::new(services::agents::ForgeAgentService {
        db: Arc::clone(&db),
        secrets: Arc::clone(&secrets),
        log_hub: Arc::clone(&log_hub),
        actions_root: actions_root.clone(),
    })
    .max_decoding_message_size(max_msg)
    .max_encoding_message_size(max_msg);

    // Raise HTTP/2 flow-control windows from the 65 KB default so a single
    // stream can saturate a fast LAN link without stalling on window updates.
    let mut builder = Server::builder()
        .initial_connection_window_size(Some(16 * 1024 * 1024))
        .initial_stream_window_size(Some(16 * 1024 * 1024))
        .tcp_nodelay(true);
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

        // On Windows, push the CA into the system trust store so clients
        // using the OS root set (forge-web's gRPC channel, browsers, curl)
        // stop tripping on our self-signed chain. No-op elsewhere.
        #[cfg(windows)]
        cert_install::ensure_ca_trusted(&paths.ca_cert);

        // Publish the full cert bundle (CA + leaf + key) to a well-known
        // shared path. Two things fall out of this:
        //
        //   1. forge-web's gRPC client auto-discovers the CA and pins it
        //      as its sole TLS trust root — no OS-trust-store dance.
        //   2. forge-web's HTTPS listener reuses the SAME leaf + key for
        //      serving browsers, so there's one cert to trust instead of
        //      two separate CAs.
        //
        // See `forge_core::ca_publish` for the target-dir fallback chain
        // and the security caveat around key readability.
        if paths.ca_cert.exists() && paths.leaf_cert.exists() && paths.leaf_key.exists() {
            let _ = forge_core::ca_publish::publish_bundle(
                &paths.ca_cert,
                &paths.leaf_cert,
                &paths.leaf_key,
            );
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
        // AgentService carries its own per-message (agent_id, token)
        // credentials verified against Argon2-hashed agent tokens in DB;
        // it deliberately bypasses the user PAT interceptor so agents
        // don't need a user account.
        .add_service(agent_svc)
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
    // Same config-relative resolution as load_serve_config so admin
    // commands operate on the *same* storage root the server uses.
    if config_path.exists() {
        resolve_base_path_relative_to_config(&mut config, config_path);
    }
    if let Some(ref storage) = cli.storage {
        config.storage.base_path = storage.into();
    }
    Ok(config)
}

/// Copy the bundled `actions/` tree into `<base>/actions/`. Only overwrites
/// files whose contents actually changed so operator overrides dropped
/// directly under `<base>/actions/` survive restarts.
fn sync_bundled_actions(dest_root: &std::path::Path) -> anyhow::Result<()> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let candidates: Vec<std::path::PathBuf> = exe_dir
        .iter()
        .map(|d| d.join("actions"))
        .chain(
            std::env::current_dir()
                .ok()
                .into_iter()
                .map(|c| c.join("actions")),
        )
        .chain(Some(std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../actions"
        ))))
        .collect();
    let source = match candidates.into_iter().find(|p| p.exists()) {
        Some(p) => p,
        None => {
            std::fs::create_dir_all(dest_root).ok();
            return Ok(());
        }
    };

    fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let path = entry.path();
            let to = dst.join(entry.file_name());
            if path.is_dir() {
                copy_dir(&path, &to)?;
            } else {
                let changed = match std::fs::read(&to) {
                    Ok(existing) => existing != std::fs::read(&path)?,
                    Err(_) => true,
                };
                if changed {
                    std::fs::copy(&path, &to)?;
                }
            }
        }
        Ok(())
    }
    copy_dir(&source, dest_root)
}
