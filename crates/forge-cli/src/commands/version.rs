// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! `forge version` (alias: `forge info`) — print the client version
//! banner and, when run inside a workspace, the server version too.
//!
//! Design notes:
//!
//! * The leading banner is intentionally identical to what `forge
//!   --version` prints. Clap drives `--version` from `long_version` on
//!   the parser, which starts with the bare version number; we prepend
//!   "forge " here to match clap's auto-prefixed output. Keeping the
//!   two in lockstep avoids the "which command tells the truth?"
//!   confusion users hit when one form showed "forge 0.1.0" and the
//!   other showed "0.2.0 / Copyright …".
//!
//! * Outside a workspace, the command stays completely silent about the
//!   missing workspace. The user ran `forge version` — they're asking
//!   "what version am I using?", not "please validate my repo".
//!
//! * Inside a workspace, we try to fetch the server version via
//!   `GetServerInfo`. A network/auth failure downgrades the output to
//!   `server (unreachable: <url>)` rather than erroring out, because
//!   the client version is still useful information.
//!
//! * `--json` is honored via the global `cli.json` flag for tooling.

use anyhow::Result;
use forge_core::workspace::Workspace;
use forge_proto::forge::GetServerInfoRequest;
use tokio::runtime::Runtime;

use crate::client;
use crate::url_resolver;

/// Version of the `forge` CLI binary, baked in at compile time from the
/// crate metadata.
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Multi-line banner printed by both `forge --version` (via clap's
/// `long_version`) and this subcommand. Kept in one place so the two
/// forms can't drift.
pub fn render_banner() -> String {
    format!(
        "forge {CLIENT_VERSION}\n\
         Copyright (c) 2026 Krishna Teja Mekala \u{2014} https://github.com/kasaiarashi/forge\n\
         Licensed under BSL 1.1",
    )
}

pub fn run(json: bool) -> Result<()> {
    // Best-effort server info lookup. Any step can fail silently — none
    // of them should turn a `forge version` call into a hard error.
    let server_info = discover_server_info();

    if json {
        let obj = serde_json::json!({
            "client": CLIENT_VERSION,
            "server": server_info.as_ref().map(|s| s.version.clone()),
            "server_url": server_info.as_ref().map(|s| s.url.clone()),
            "server_error": server_info.as_ref().and_then(|s| s.error.clone()),
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
        return Ok(());
    }

    println!("{}", render_banner());
    if let Some(info) = server_info {
        match (info.version.is_empty(), info.error) {
            (false, _) => println!("server {} ({})", info.version, info.url),
            (true, Some(err)) => println!("server (unreachable: {}) — {err}", info.url),
            (true, None) => println!("server (unreachable: {})", info.url),
        }
    }
    Ok(())
}

/// What we managed to learn about the upstream server.
struct ServerInfo {
    url: String,
    version: String,
    error: Option<String>,
}

/// Probe the current directory for a workspace, pull its default remote
/// URL, and try a `GetServerInfo` RPC. Returns `None` when there's no
/// workspace at all or when the workspace has no remote configured —
/// those are the "no repo / no error" cases the user asked for.
fn discover_server_info() -> Option<ServerInfo> {
    let cwd = std::env::current_dir().ok()?;
    let ws = Workspace::discover(&cwd).ok()?;
    let cfg = ws.config().ok()?;
    let url = cfg.default_remote_url()?.to_string();

    // Build a fresh tokio runtime just for this one call. Version is a
    // one-shot command so runtime reuse isn't worth the complexity.
    let rt = Runtime::new().ok()?;
    let result = rt.block_on(async {
        // Auto-switch web → gRPC URLs so users can point at either.
        let resolved = url_resolver::resolve(&url).await;
        let mut client = client::connect_forge(&resolved).await?;
        let resp = client
            .get_server_info(GetServerInfoRequest {})
            .await?
            .into_inner();
        anyhow::Ok(resp.version)
    });

    match result {
        Ok(version) => Some(ServerInfo {
            url,
            version,
            error: None,
        }),
        Err(e) => Some(ServerInfo {
            url,
            version: String::new(),
            error: Some(short_error(&e)),
        }),
    }
}

/// Collapse an error chain to its tail (the actual cause) so the one-line
/// version printout stays readable. `tonic::Status` Display is useful;
/// nested transport errors aren't.
fn short_error(err: &anyhow::Error) -> String {
    let mut last = err.to_string();
    let mut source: Option<&dyn std::error::Error> = err.source();
    while let Some(s) = source {
        last = s.to_string();
        source = s.source();
    }
    last
}
