// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! `forge-agent` — distributed CI runner.
//!
//! A forge-agent host pulls queued workflow runs from a forge-server over
//! gRPC, executes the steps in a local workspace, and streams logs back.
//! The agent is stateless aside from its cached auth token and an action
//! YAML cache — everything authoritative lives on the server.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{error, info, warn};

use forge_proto::forge::agent_service_client::AgentServiceClient;
use forge_proto::forge::*;

mod runner;

#[derive(Parser)]
#[command(name = "forge-agent", about = "Forge CI agent", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Store agent credentials + labels to a config file.
    Register {
        #[arg(long)]
        server: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        token: String,
        #[arg(long, num_args = 0..)]
        labels: Vec<String>,
        /// Output config path. Default: ./forge-agent.toml.
        #[arg(long, default_value = "forge-agent.toml")]
        config: PathBuf,
    },
    /// Run the claim loop. Uses the config written by `register`.
    Run {
        #[arg(long, default_value = "forge-agent.toml")]
        config: PathBuf,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct AgentConfig {
    server: String,
    name: String,
    token: String,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default = "default_work_dir")]
    work_dir: PathBuf,
    #[serde(default = "default_max_concurrent")]
    max_concurrent: u32,
}

fn default_work_dir() -> PathBuf {
    PathBuf::from("forge-agent-work")
}
fn default_max_concurrent() -> u32 {
    1
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    // Pin a rustls provider up-front. Same rationale as forge-server/cli.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let cli = Cli::parse();
    match cli.command {
        Command::Register {
            server,
            name,
            token,
            labels,
            config,
        } => register(server, name, token, labels, config),
        Command::Run { config } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(run_agent(config))
        }
    }
}

fn register(
    server: String,
    name: String,
    token: String,
    labels: Vec<String>,
    config_path: PathBuf,
) -> Result<()> {
    if config_path.exists() {
        anyhow::bail!(
            "config file already exists at {}; delete it first to re-register",
            config_path.display()
        );
    }
    let cfg = AgentConfig {
        server,
        name,
        token,
        labels,
        work_dir: default_work_dir(),
        max_concurrent: default_max_concurrent(),
    };
    let toml = toml::to_string_pretty(&cfg)?;
    std::fs::write(&config_path, toml)
        .with_context(|| format!("write {}", config_path.display()))?;
    // Lock 0600 on Unix so the token file isn't world-readable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&config_path)?.permissions();
        perms.set_mode(0o600);
        let _ = std::fs::set_permissions(&config_path, perms);
    }
    println!("Wrote {}. Start the agent with: forge-agent run", config_path.display());
    Ok(())
}

async fn run_agent(config_path: PathBuf) -> Result<()> {
    let cfg: AgentConfig = {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("read {}", config_path.display()))?;
        toml::from_str(&content)?
    };
    std::fs::create_dir_all(&cfg.work_dir)?;

    info!(server = %cfg.server, name = %cfg.name, "forge-agent starting");

    // Connect. Agents accept the server's TLS chain via whatever the
    // operator trusted in the OS store (or the forge-server CA the
    // installer pinned). tonic's `tls_config` defaults handle that.
    let endpoint = tonic::transport::Endpoint::from_shared(cfg.server.clone())
        .map_err(|e| anyhow!("bad server URL: {e}"))?
        .keep_alive_while_idle(true)
        .http2_keep_alive_interval(std::time::Duration::from_secs(20))
        .keep_alive_timeout(std::time::Duration::from_secs(10));

    let endpoint = if cfg.server.starts_with("https://") {
        endpoint.tls_config(tonic::transport::ClientTlsConfig::new().with_enabled_roots())?
    } else {
        endpoint
    };
    let channel = endpoint.connect().await.context("connect to server")?;
    let mut client = AgentServiceClient::new(channel.clone());

    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "unknown"
    };

    let reg = client
        .register_agent(RegisterAgentRequest {
            name: cfg.name.clone(),
            token: cfg.token.clone(),
            labels: cfg.labels.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: os.to_string(),
        })
        .await
        .context("register_agent")?
        .into_inner();
    info!(agent_id = reg.agent_id, "registered with server");

    let heartbeat_handle = {
        let cfg = cfg.clone_for_task();
        let mut client = client.clone();
        let id = reg.agent_id;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(
                reg.heartbeat_seconds.max(5) as u64,
            ));
            tick.tick().await; // skip immediate
            loop {
                tick.tick().await;
                let _ = client
                    .heartbeat(HeartbeatRequest {
                        agent_id: id,
                        token: cfg.token.clone(),
                    })
                    .await;
            }
        })
    };

    // Claim loop.
    let agent_id = reg.agent_id;
    let poll_seconds = reg.claim_poll_seconds.max(5);
    let cfg = Arc::new(cfg);
    loop {
        let claim = client
            .claim_job(ClaimJobRequest {
                agent_id,
                token: cfg.token.clone(),
                wait_seconds: poll_seconds,
            })
            .await;
        match claim {
            Ok(resp) => {
                let resp = resp.into_inner();
                if resp.run_id == 0 {
                    continue;
                }
                info!(run_id = resp.run_id, "claimed run");
                if let Err(e) =
                    runner::execute_run(&mut client.clone(), Arc::clone(&cfg), agent_id, resp)
                        .await
                {
                    error!(error = %e, "run execution failed");
                }
            }
            Err(e) => {
                warn!(error = %e, "claim_job failed, backing off 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
    // heartbeat_handle is intentionally left running for the loop's life.
    #[allow(unreachable_code)]
    {
        drop(heartbeat_handle);
        Ok(())
    }
}

impl AgentConfig {
    fn clone_for_task(&self) -> Self {
        Self {
            server: self.server.clone(),
            name: self.name.clone(),
            token: self.token.clone(),
            labels: self.labels.clone(),
            work_dir: self.work_dir.clone(),
            max_concurrent: self.max_concurrent,
        }
    }
}

#[allow(dead_code)]
fn touch(_: &Path) {}
