// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

mod api;
mod api_actions;
mod auth;
mod config;
mod grpc_client;
mod tls_autogen;

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{header, HeaderValue};
use axum::middleware;
use axum::routing::{delete, get, post, put};
use axum::Router;
use clap::{Parser, Subcommand};
use tokio::sync::RwLock;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::GovernorLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

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

/// `GET /.well-known/forge-server-info` handler. Returns the upstream
/// forge-server's gRPC scheme and port so the `forge` CLI can transparently
/// switch when a user points it at the web URL by mistake.
///
/// The response deliberately omits the host: the CLI reuses whatever host
/// it reached forge-web on. This way it works across LAN / VPN / hostname
/// aliases without the server having to guess which of its many names the
/// client can reach it at.
async fn well_known_server_info(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<AppState>>,
) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "service": "forge-web",
        "grpc_scheme": state.grpc_scheme,
        "grpc_port": state.grpc_port,
    }))
}

/// Parse a gRPC URL like `https://127.0.0.1:9876` or `http://forge:9876`
/// into its `(scheme, port)` pair. Returns `None` on malformed input —
/// the caller falls back to sensible defaults.
fn parse_grpc_scheme_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = url.strip_prefix("http://") {
        ("http", r)
    } else {
        return None;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let port = match authority.rsplit_once(':') {
        Some((_, p)) => p.parse().ok()?,
        None => {
            // Default ports when the URL omits them — unusual for gRPC
            // but we handle it anyway.
            if scheme == "https" {
                443
            } else {
                80
            }
        }
    };
    Some((scheme.to_string(), port))
}

/// Enumerate every non-loopback, non-link-local interface IP. Used for the
/// auto-TLS SAN when forge-web binds to 0.0.0.0. Twin of the helper in
/// forge-server/src/main.rs.
fn local_non_loopback_ips() -> Vec<std::net::IpAddr> {
    match if_addrs::get_if_addrs() {
        Ok(addrs) => addrs
            .into_iter()
            .filter_map(|iface| {
                let ip = iface.ip();
                if ip.is_loopback() {
                    return None;
                }
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

pub struct AppState {
    pub config: Config,
    /// True if session cookies should carry the `Secure` attribute. Computed
    /// at startup: true when either forge-web terminates TLS itself OR the
    /// operator explicitly opted in via `web.secure_cookies = true`.
    pub secure_cookies: bool,
    /// Parsed (scheme, port) of the upstream forge-server gRPC URL, exposed
    /// to clients via `/.well-known/forge-server-info` so the `forge` CLI
    /// can auto-switch when given the web URL instead of the gRPC URL.
    pub grpc_scheme: String,
    pub grpc_port: u16,
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
        let client = ForgeGrpcClient::connect(
            &self.config.server.grpc_url,
            self.config.server.ca_cert_path.as_deref(),
        )
        .await?;
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

    /// Build a fresh `AuthServiceClient` with NO bearer token attached. The
    /// login / bootstrap / is_initialized endpoints have to work for users
    /// who have a stale or invalid cookie — forwarding that cookie would
    /// make forge-server reject the call as Unauthenticated before the
    /// handler even ran.
    pub async fn grpc_auth_client_anonymous(
        &self,
    ) -> anyhow::Result<
        forge_proto::forge::auth_service_client::AuthServiceClient<tonic::transport::Channel>,
    > {
        let client = self.grpc_client().await?;
        Ok(client.auth_anonymous())
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Install a rustls crypto provider up-front. See twin comment in
    // forge-server/src/main.rs — rustls refuses to auto-select when more
    // than one provider crate is linked in, so we pick aws-lc-rs.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

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
    let tls_cfg = cfg.web.tls.clone();
    let rate_limit = cfg.web.rate_limit.clone();
    // Secure cookie attribute: on when forge-web terminates TLS itself OR
    // when the operator explicitly opted in via `secure_cookies = true`. A
    // loopback plaintext dev server is the only scenario where this should
    // be off.
    let secure_cookies = cfg.web.secure_cookies || tls_cfg.is_some();
    if !secure_cookies {
        tracing::warn!(
            "secure_cookies is disabled AND no TLS configured — session \
             cookies will be sent in the clear. Only acceptable for loopback dev."
        );
    }

    // Parse the upstream gRPC URL so we can advertise it at the
    // well-known endpoint. The scheme ("http" / "https") and the port are
    // the only useful parts — we deliberately don't publish the host,
    // since the client uses whatever host it reached forge-web on.
    let (grpc_scheme, grpc_port) = parse_grpc_scheme_port(&cfg.server.grpc_url)
        .unwrap_or_else(|| ("https".to_string(), 9876));

    let state = Arc::new(AppState {
        config: cfg,
        secure_cookies,
        grpc_scheme,
        grpc_port,
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

    // (api_routes assembled below after rate-limit layer is attached.)

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

    // CORS: default to *no* cross-origin requests (the SPA is same-origin).
    // Only when `allowed_origins` is explicitly configured do we emit CORS
    // headers, and then only for the listed origins. `mirror_request` +
    // `allow_credentials` is never set — that combination lets any page
    // on the internet issue credentialed requests to this API.
    let cors = if allowed_origins.is_empty() {
        None
    } else {
        let origins: Vec<HeaderValue> = allowed_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        if origins.is_empty() {
            tracing::warn!(
                "allowed_origins was non-empty but no entries parsed as valid \
                 header values; CORS will be disabled"
            );
            None
        } else {
            Some(
                CorsLayer::new()
                    .allow_methods([
                        axum::http::Method::GET,
                        axum::http::Method::POST,
                        axum::http::Method::PUT,
                        axum::http::Method::DELETE,
                        axum::http::Method::OPTIONS,
                    ])
                    .allow_headers([
                        header::CONTENT_TYPE,
                        header::AUTHORIZATION,
                    ])
                    .allow_credentials(true)
                    .allow_origin(AllowOrigin::list(origins)),
            )
        }
    };

    // Security response headers. Defense in depth against MIME sniffing,
    // clickjacking, referrer leakage, and embedded content. HSTS is only
    // emitted when we're actually terminating TLS; otherwise it would
    // permanently upgrade clients to HTTPS on a server that can't speak it.
    let sec_headers = tower::ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; img-src 'self' data:; \
                 style-src 'self' 'unsafe-inline'; \
                 script-src 'self'; \
                 connect-src 'self'; \
                 frame-ancestors 'none'; \
                 base-uri 'self'; \
                 form-action 'self'",
            ),
        ));

    let hsts_layer = tls_cfg.as_ref().map(|_| {
        SetResponseHeaderLayer::if_not_present(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        )
    });

    // Rate limit layer — applied only to /api/auth/* so legitimate CI/CD
    // clients aren't throttled on push/pull hot paths. Keyed on the peer IP
    // (SmartIpKeyExtractor honors X-Forwarded-For only if set by a trusted
    // proxy, which we don't here — so it defaults to the connecting socket).
    let auth_rate_limiter = GovernorConfigBuilder::default()
        .per_second(rate_limit.per_second)
        .burst_size(rate_limit.burst)
        .finish()
        .ok_or_else(|| anyhow::anyhow!("invalid rate_limit config"))?;
    let auth_rate_layer = GovernorLayer {
        config: Arc::new(auth_rate_limiter),
    };

    // Wrap auth_routes in rate limiting before nesting.
    let auth_routes = auth_routes.layer(auth_rate_layer);

    let api_routes = Router::new()
        .nest("/auth", auth_routes)
        .merge(public_api)
        .merge(protected_api);

    let mut app = Router::new()
        // Well-known server-info endpoint. Unauthenticated and rate-limit
        // exempt so the `forge` CLI can probe it cheaply to tell "web URL"
        // apart from "gRPC URL" without needing a valid session. Returns
        // the upstream gRPC scheme+port the CLI should switch to.
        .route(
            "/.well-known/forge-server-info",
            get(well_known_server_info),
        )
        .nest("/api", api_routes)
        .fallback_service(spa_service)
        // Run every request inside the session-token task-local scope so the
        // gRPC client can read the cookie's session token without rewriting
        // every handler signature. Layer order matters: cors must wrap the
        // session layer because the cors preflight responses don't need a
        // token. The session layer wraps with_state so handlers see the
        // task-local already populated.
        .layer(middleware::from_fn(auth::session_token_layer))
        .layer(sec_headers);

    if let Some(layer) = hsts_layer {
        app = app.layer(layer);
    }
    if let Some(cors_layer) = cors {
        app = app.layer(cors_layer);
    }
    let app = app.with_state(state);

    // ---- Start server ----
    let addr: std::net::SocketAddr = listen_addr
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid listen address '{listen_addr}': {e}"))?;

    match tls_cfg {
        Some(tls) => {
            // Resolve cert/key paths, with auto-generate fallback under
            // ./forge-web-certs/ if the operator didn't provide paths.
            let cert_base = std::path::Path::new("./forge-web-certs");
            let defaults = tls_autogen::TlsPaths::under(cert_base);
            let paths = tls_autogen::TlsPaths {
                ca_cert: defaults.ca_cert.clone(),
                ca_key: defaults.ca_key.clone(),
                leaf_cert: tls.cert_path.clone().unwrap_or(defaults.leaf_cert),
                leaf_key: tls.key_path.clone().unwrap_or(defaults.leaf_key),
            };
            if tls.auto_generate {
                let mut sans = tls.hostnames.clone();
                let listen_ip = addr.ip();
                if listen_ip.is_unspecified() {
                    // Binding to 0.0.0.0 / :: — enumerate every reachable
                    // non-loopback interface IP so LAN clients don't hit a
                    // SAN mismatch without extra config.
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
                    .map_err(|e| anyhow::anyhow!("auto-generating TLS: {e}"))?;
            }

            tracing::info!("forge-web listening on https://{addr}");
            if let Some(fp) = tls_autogen::cert_fingerprint(&paths.ca_cert) {
                tracing::warn!(
                    "\n*** forge-web CA fingerprint (SHA-256):\n***   {fp}\n\
                     *** Import {} into your OS trust store to remove the \
                     browser warning.",
                    paths.ca_cert.display()
                );
            }

            let rustls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
                &paths.leaf_cert,
                &paths.leaf_key,
            )
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "failed to load TLS cert/key ({:?}, {:?}): {e}",
                    paths.leaf_cert,
                    paths.leaf_key
                )
            })?;
            axum_server::bind_rustls(addr, rustls)
                .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await?;
        }
        None => {
            tracing::info!("forge-web listening on http://{addr} (plaintext)");
            axum_server::bind(addr)
                .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await?;
        }
    }

    Ok(())
}
