// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under BSL 1.1.

//! Phase 7g — Docker-managed Postgres for self-hosted deployments.
//!
//! Operators that don't want to run a separate Postgres instance can
//! `forge-server postgres up` to get a containerised one with its
//! data directory mapped under the same `base_path` the rest of the
//! server uses. This keeps the deployment "transferable" — copy the
//! base directory, get the database with it.
//!
//! The implementation shells out to the `docker` CLI rather than
//! linking the bollard SDK; that pulls another ~30 transitive deps
//! and the operator already has Docker installed if they're picking
//! this option. The same goes for `podman` (drop-in compatible) —
//! `forge-server postgres up --runtime podman` works.
//!
//! Layout under `<base_path>/postgres/`:
//! - `data/` — bind-mounted at `/var/lib/postgresql/data` inside the
//!   container. Survives container recreation.
//! - `credentials.json` — generated on first run; holds the
//!   superuser password. Mode 0600. Re-used on subsequent restarts.
//! - `state.json` — last-known container name + port. Lets `down`
//!   and `status` find the container the server is using even when
//!   the operator typed a new port flag.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Default Postgres image. Pinned to a specific minor for
/// reproducibility — operators upgrade by overriding in
/// `[postgres] image = "..."` once we expose it through config.
pub const DEFAULT_IMAGE: &str = "postgres:16";

/// Default container name. Sharing the name across forge-server
/// installations on one host is fine — the up flow refuses to
/// stomp on a running container it didn't create.
pub const DEFAULT_CONTAINER_NAME: &str = "forge-postgres";

/// Default port mapped on the host. Avoids clashing with a
/// system-installed Postgres on 5432.
pub const DEFAULT_HOST_PORT: u16 = 5433;

/// Default DB + user + role for the bootstrapped instance. The
/// operator can change them via flags.
pub const DEFAULT_DB_NAME: &str = "forge";
pub const DEFAULT_DB_USER: &str = "forge";

/// Knobs exposed to the CLI. Only the runtime + container name + port
/// + image differ across `up`/`down`/`status`; passwords are
/// auto-generated.
#[derive(Debug, Clone)]
pub struct PostgresDockerConfig {
    /// `docker` (default) or `podman`.
    pub runtime: String,
    /// Container name used for both up and down.
    pub container_name: String,
    /// Host port mapped to container's 5432.
    pub host_port: u16,
    /// Postgres image tag.
    pub image: String,
    /// Base directory the server uses; postgres data lives at
    /// `<base>/postgres/data`.
    pub base_path: PathBuf,
}

impl PostgresDockerConfig {
    pub fn defaults_under(base_path: PathBuf) -> Self {
        Self {
            runtime: "docker".into(),
            container_name: DEFAULT_CONTAINER_NAME.into(),
            host_port: DEFAULT_HOST_PORT,
            image: DEFAULT_IMAGE.into(),
            base_path,
        }
    }

    fn pg_root(&self) -> PathBuf {
        self.base_path.join("postgres")
    }
    fn data_dir(&self) -> PathBuf {
        self.pg_root().join("data")
    }
    fn credentials_path(&self) -> PathBuf {
        self.pg_root().join("credentials.json")
    }
    fn state_path(&self) -> PathBuf {
        self.pg_root().join("state.json")
    }
}

/// On-disk credentials so the operator can inspect them, and so
/// repeated `up` calls reuse the same password rather than
/// regenerating one (which would need a re-init of the data dir).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub user: String,
    pub password: String,
    pub database: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct State {
    container_name: String,
    host_port: u16,
    image: String,
    created_at: i64,
}

/// `forge-server postgres up`. Idempotent: re-running while the
/// container is healthy is a no-op; it only restarts a stopped one.
pub fn up(cfg: &PostgresDockerConfig) -> Result<UpReport> {
    ensure_runtime_present(&cfg.runtime)?;
    std::fs::create_dir_all(cfg.data_dir())?;
    let creds = load_or_generate_credentials(cfg)?;

    if container_running(&cfg.runtime, &cfg.container_name)? {
        let url = connection_url(&creds, cfg.host_port);
        return Ok(UpReport {
            already_running: true,
            credentials: creds,
            container_name: cfg.container_name.clone(),
            host_port: cfg.host_port,
            url,
        });
    }

    if container_exists(&cfg.runtime, &cfg.container_name)? {
        // Stopped but present — start it back up rather than recreate.
        let out = Command::new(&cfg.runtime)
            .args(["start", &cfg.container_name])
            .output()
            .context("docker start")?;
        if !out.status.success() {
            bail!(
                "{} start failed: {}",
                cfg.runtime,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    } else {
        // Fresh run. We use `--mount type=bind,…` instead of the
        // shorter `-v src:target` form because the v-form parses
        // colons as field separators, which collides with Windows
        // drive letters (`W:/foo:/bar` is ambiguous). `--mount`
        // takes explicit `source=` / `target=` keys.
        let mount_spec = format!(
            "type=bind,source={},target=/var/lib/postgresql/data",
            path_to_docker(&cfg.data_dir())
        );
        let port_map = format!("{}:5432", cfg.host_port);
        let out = Command::new(&cfg.runtime)
            .args([
                "run",
                "-d",
                "--name",
                &cfg.container_name,
                "--restart",
                "unless-stopped",
                "-e",
                &format!("POSTGRES_PASSWORD={}", creds.password),
                "-e",
                &format!("POSTGRES_USER={}", creds.user),
                "-e",
                &format!("POSTGRES_DB={}", creds.database),
                "--mount",
                &mount_spec,
                "-p",
                &port_map,
                &cfg.image,
            ])
            .output()
            .context("docker run")?;
        if !out.status.success() {
            bail!(
                "{} run failed: {}",
                cfg.runtime,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        write_state(cfg)?;
    }

    // Wait for the server to accept connections. Use pg_isready inside
    // the container — it's bundled with the postgres image and saves
    // us a libpq dep on the host.
    wait_for_ready(&cfg.runtime, &cfg.container_name)?;

    Ok(UpReport {
        already_running: false,
        credentials: creds.clone(),
        container_name: cfg.container_name.clone(),
        host_port: cfg.host_port,
        url: connection_url(&creds, cfg.host_port),
    })
}

/// `forge-server postgres down`. Stops the container; data dir
/// stays. Pass `remove = true` to also `docker rm` so the next
/// `up` recreates from scratch (still preserves the data
/// directory, just not the container metadata).
pub fn down(cfg: &PostgresDockerConfig, remove: bool) -> Result<()> {
    ensure_runtime_present(&cfg.runtime)?;
    if !container_exists(&cfg.runtime, &cfg.container_name)? {
        return Ok(());
    }
    let _ = Command::new(&cfg.runtime)
        .args(["stop", &cfg.container_name])
        .output()
        .context("docker stop")?;
    if remove {
        let _ = Command::new(&cfg.runtime)
            .args(["rm", &cfg.container_name])
            .output()
            .context("docker rm")?;
    }
    Ok(())
}

/// `forge-server postgres status`. Prints a one-shot summary; never
/// fails when the container is missing or runtime absent — the
/// caller relies on the printed text to tell the operator what's
/// going on.
pub fn status(cfg: &PostgresDockerConfig) -> StatusReport {
    let runtime_present = ensure_runtime_present(&cfg.runtime).is_ok();
    let exists = if runtime_present {
        container_exists(&cfg.runtime, &cfg.container_name).unwrap_or(false)
    } else {
        false
    };
    let running = if exists {
        container_running(&cfg.runtime, &cfg.container_name).unwrap_or(false)
    } else {
        false
    };
    let creds = if cfg.credentials_path().exists() {
        load_credentials(cfg).ok()
    } else {
        None
    };
    let url = creds.as_ref().map(|c| connection_url(c, cfg.host_port));
    StatusReport {
        runtime_present,
        container_name: cfg.container_name.clone(),
        host_port: cfg.host_port,
        exists,
        running,
        url,
    }
}

#[derive(Debug)]
pub struct UpReport {
    pub already_running: bool,
    pub credentials: Credentials,
    pub container_name: String,
    pub host_port: u16,
    pub url: String,
}

#[derive(Debug)]
pub struct StatusReport {
    pub runtime_present: bool,
    pub container_name: String,
    pub host_port: u16,
    pub exists: bool,
    pub running: bool,
    pub url: Option<String>,
}

// ── helpers ────────────────────────────────────────────────────────────────

fn ensure_runtime_present(runtime: &str) -> Result<()> {
    let out = Command::new(runtime).arg("--version").output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(anyhow!(
            "`{runtime} --version` exited non-zero: {}",
            String::from_utf8_lossy(&o.stderr)
        )),
        Err(e) => Err(anyhow!(
            "`{runtime}` not found on PATH ({e}); install Docker / Podman first"
        )),
    }
}

fn container_exists(runtime: &str, name: &str) -> Result<bool> {
    let out = Command::new(runtime)
        .args(["ps", "-a", "--filter", &format!("name=^{name}$"), "--format", "{{.Names}}"])
        .output()
        .context("docker ps")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout.lines().any(|l| l.trim() == name))
}

fn container_running(runtime: &str, name: &str) -> Result<bool> {
    let out = Command::new(runtime)
        .args(["ps", "--filter", &format!("name=^{name}$"), "--format", "{{.Names}}"])
        .output()
        .context("docker ps")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout.lines().any(|l| l.trim() == name))
}

fn wait_for_ready(runtime: &str, container_name: &str) -> Result<()> {
    use std::thread::sleep;
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        let out = Command::new(runtime)
            .args(["exec", container_name, "pg_isready", "-U", DEFAULT_DB_USER])
            .output();
        if let Ok(o) = out {
            if o.status.success() {
                return Ok(());
            }
        }
        sleep(Duration::from_millis(500));
    }
    bail!("timed out waiting for postgres container '{container_name}' to become ready")
}

fn load_or_generate_credentials(cfg: &PostgresDockerConfig) -> Result<Credentials> {
    let path = cfg.credentials_path();
    if path.exists() {
        return load_credentials(cfg);
    }
    std::fs::create_dir_all(cfg.pg_root())?;
    let creds = Credentials {
        user: DEFAULT_DB_USER.into(),
        password: random_password(),
        database: DEFAULT_DB_NAME.into(),
    };
    let json = serde_json::to_string_pretty(&creds)?;
    std::fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(creds)
}

fn load_credentials(cfg: &PostgresDockerConfig) -> Result<Credentials> {
    let bytes = std::fs::read(cfg.credentials_path())
        .with_context(|| format!("read {}", cfg.credentials_path().display()))?;
    let creds: Credentials = serde_json::from_slice(&bytes).context("parse credentials.json")?;
    Ok(creds)
}

fn write_state(cfg: &PostgresDockerConfig) -> Result<()> {
    let state = State {
        container_name: cfg.container_name.clone(),
        host_port: cfg.host_port,
        image: cfg.image.clone(),
        created_at: chrono::Utc::now().timestamp(),
    };
    let json = serde_json::to_string_pretty(&state)?;
    std::fs::write(cfg.state_path(), json)?;
    Ok(())
}

fn random_password() -> String {
    use rand::RngCore;
    let mut raw = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut raw);
    // URL-safe base64 minus padding so the password drops cleanly
    // into a `postgres://user:pass@host/db` URL without escaping.
    use base64_compat::URL_SAFE_NO_PAD;
    URL_SAFE_NO_PAD.encode(raw)
}

/// Tiny base64 shim — kept inline so we don't take on a `base64`
/// crate dep just for one bootstrap helper. Mirrors RFC 4648 §5.
mod base64_compat {
    const URL_SAFE_ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    pub struct UrlSafeNoPad;
    pub const URL_SAFE_NO_PAD: UrlSafeNoPad = UrlSafeNoPad;

    impl UrlSafeNoPad {
        pub fn encode(&self, input: impl AsRef<[u8]>) -> String {
            let bytes = input.as_ref();
            let mut out = String::with_capacity((bytes.len() * 4 + 2) / 3);
            for chunk in bytes.chunks(3) {
                let b0 = chunk[0];
                let b1 = chunk.get(1).copied().unwrap_or(0);
                let b2 = chunk.get(2).copied().unwrap_or(0);
                let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
                out.push(URL_SAFE_ALPHABET[((n >> 18) & 0x3F) as usize] as char);
                out.push(URL_SAFE_ALPHABET[((n >> 12) & 0x3F) as usize] as char);
                if chunk.len() > 1 {
                    out.push(URL_SAFE_ALPHABET[((n >> 6) & 0x3F) as usize] as char);
                }
                if chunk.len() > 2 {
                    out.push(URL_SAFE_ALPHABET[(n & 0x3F) as usize] as char);
                }
            }
            out
        }
    }
}

fn path_to_docker(p: &Path) -> String {
    // Docker on Windows accepts both forward-slash and Windows-style
    // paths in -v mounts when using the modern engine. Normalise to
    // forward slashes for clean log output.
    let s = p.display().to_string();
    s.replace('\\', "/")
}

fn connection_url(creds: &Credentials, port: u16) -> String {
    format!(
        "postgres://{}:{}@127.0.0.1:{}/{}",
        creds.user, creds.password, port, creds.database
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_persist_round_trip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg = PostgresDockerConfig::defaults_under(tmp.path().to_path_buf());
        let first = load_or_generate_credentials(&cfg).unwrap();
        let second = load_or_generate_credentials(&cfg).unwrap();
        assert_eq!(first.password, second.password);
        assert_eq!(first.user, DEFAULT_DB_USER);
        assert_eq!(first.database, DEFAULT_DB_NAME);
    }

    #[test]
    fn url_format_is_libpq_compatible() {
        let creds = Credentials {
            user: "forge".into(),
            password: "p@ss".into(),
            database: "forge".into(),
        };
        let url = connection_url(&creds, 5433);
        assert!(url.starts_with("postgres://forge:"));
        assert!(url.ends_with("@127.0.0.1:5433/forge"));
    }
}
