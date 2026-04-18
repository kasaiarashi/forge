// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Windows service integration for forge-server.
//!
//! Two surfaces:
//!
//! 1. **`forge-server service install/uninstall/start/stop`** — operator-
//!    facing CLI that talks to the Windows Service Control Manager. The
//!    Inno Setup installer calls `install` after laying down the binaries
//!    and `uninstall` from its uninstall step.
//!
//! 2. **`forge-server --as-service serve`** — what the SCM actually invokes
//!    when starting the service. The flag is hidden from `--help`. When
//!    set, [`run_under_scm`] hands the process over to the SCM dispatcher
//!    which then calls back into [`service_main`] → [`run_service`] which
//!    spins up the same Tokio runtime + serve loop the interactive case
//!    uses, but plumbs an SCM-driven shutdown signal into
//!    `Server::serve_with_shutdown`.
//!
//! Cross-platform note: this module is gated by `#[cfg(windows)]` and is
//! never compiled on Linux/macOS. main.rs guards every call into here
//! with the same cfg.

#![cfg(windows)]

use anyhow::{anyhow, Context, Result};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

use crate::config::ServerConfig;

/// SCM identifier. Must match the name used by the installer when it
/// calls `install`. Don't change without bumping a migration step in
/// `installers/windows/forge-server.iss` or operators will end up with
/// orphaned old-name services on upgrade.
pub const SERVICE_NAME: &str = "ForgeServer";
pub const DISPLAY_NAME: &str = "Forge VCS Server";
pub const DESCRIPTION: &str =
    "Forge VCS gRPC server — binary-first version control for game teams.";

const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

/// Stash the parsed config + the shared bootstrap state before handing
/// control to the SCM dispatcher. `service_main` is a free function (the
/// `define_windows_service!` macro can't capture closures) so the only
/// way to pass data into it is through a static. We're inside a single
/// process so the OnceLock is fine.
static SERVICE_PAYLOAD: OnceLock<ServicePayload> = OnceLock::new();

/// Everything `run_service` needs to spin up the server. Constructed once
/// in `main` and frozen via [`SERVICE_PAYLOAD`].
pub struct ServicePayload {
    pub config: ServerConfig,
}

/// Hand control to the Windows SCM. The function blocks until the service
/// stops; on a non-SCM context (someone running `forge-server --as-service
/// serve` from a regular shell) it returns an error containing
/// `ERROR_FAILED_SERVICE_CONTROLLER_CONNECT` (1063) almost immediately,
/// and main.rs falls back to running interactively.
pub fn run_under_scm(payload: ServicePayload) -> Result<()> {
    SERVICE_PAYLOAD
        .set(payload)
        .map_err(|_| anyhow!("service payload already set"))?;
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|e| anyhow!("service dispatcher: {e}"))
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service() {
        // No reliable logger here — the SCM is the only thing watching
        // stderr, and it routes the output to the system event log.
        eprintln!("forge-server service exited with error: {e:#}");
    }
}

fn run_service() -> Result<()> {
    let payload = SERVICE_PAYLOAD
        .get()
        .ok_or_else(|| anyhow!("service payload missing — run_under_scm not called"))?;

    // Channel used by the SCM control handler to forward Stop/Shutdown
    // signals to the tokio shutdown future. Wrapped in an Arc<Mutex<Option>>
    // because the handler closure is `Fn`, not `FnOnce`.
    let stop_tx_holder: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>> =
        Arc::new(Mutex::new(None));
    let stop_tx_for_handler = stop_tx_holder.clone();

    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            ServiceControl::Stop | ServiceControl::Shutdown => {
                if let Ok(mut guard) = stop_tx_for_handler.lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(());
                    }
                }
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)
        .map_err(|e| anyhow!("register service control handler: {e}"))?;

    set_status(
        &status_handle,
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        Duration::from_secs(30),
    )?;

    // Build the runtime, install the rustls crypto provider (matches the
    // interactive path), and run the server.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    *stop_tx_holder.lock().unwrap() = Some(stop_tx);

    // Tell the SCM we're Running BEFORE we actually finish binding —
    // otherwise it might consider startup hung. The bind happens
    // microseconds later inside serve_inner.
    set_status(
        &status_handle,
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        Duration::default(),
    )?;

    let serve_result = build_runtime()?.block_on(async move {
        crate::serve_inner(payload.config.clone(), async move {
            // Resolves when the SCM event handler dropped the sender.
            let _ = stop_rx.await;
        })
        .await
    });

    let exit_code = if serve_result.is_ok() { 0 } else { 1 };
    let _ = set_status_with_exit(&status_handle, ServiceState::Stopped, exit_code);

    serve_result
}

fn build_runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("forge-server-svc")
        .build()
        .map_err(|e| anyhow!("build tokio runtime: {e}"))
}

fn set_status(
    handle: &ServiceStatusHandle,
    state: ServiceState,
    controls: ServiceControlAccept,
    wait_hint: Duration,
) -> Result<()> {
    handle
        .set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: state,
            controls_accepted: controls,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint,
            process_id: None,
        })
        .map_err(|e| anyhow!("set service status: {e}"))
}

fn set_status_with_exit(
    handle: &ServiceStatusHandle,
    state: ServiceState,
    exit_code: u32,
) -> Result<()> {
    handle
        .set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: state,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(exit_code),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
        .map_err(|e| anyhow!("set service status: {e}"))
}

// ── CLI subcommand handlers ──────────────────────────────────────────────────

/// `forge-server service install` — register the service with the SCM
/// pointing at the current binary path. Auto-start on boot, runs as
/// LocalSystem unless an account is supplied later.
pub fn install(binary_path: PathBuf, config_path: PathBuf) -> Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("connect to service manager")?;

    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: binary_path,
        // Order matters: top-level flags first, then the subcommand.
        // The `--as-service` flag is what flips main.rs into SCM mode.
        launch_arguments: vec![
            OsString::from("--config"),
            OsString::from(config_path),
            OsString::from("--as-service"),
            OsString::from("serve"),
        ],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };

    let service = manager
        .create_service(&info, ServiceAccess::CHANGE_CONFIG)
        .context("create service")?;
    service
        .set_description(DESCRIPTION)
        .context("set service description")?;
    Ok(())
}

/// `forge-server service uninstall` — best-effort stop, then delete.
/// Idempotent: returns Ok if the service is already gone.
pub fn uninstall() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("connect to service manager")?;

    let service = match manager.open_service(
        SERVICE_NAME,
        ServiceAccess::DELETE | ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
    ) {
        Ok(s) => s,
        Err(windows_service::Error::Winapi(io_err)) if io_err.raw_os_error() == Some(1060) => {
            // ERROR_SERVICE_DOES_NOT_EXIST — already uninstalled.
            return Ok(());
        }
        Err(e) => return Err(anyhow!("open service: {e}")),
    };

    // Best-effort stop. We swallow "service not running" errors and rely
    // on the small sleep below for any in-flight shutdown to wind down.
    let _ = service.stop();
    std::thread::sleep(Duration::from_secs(2));
    service.delete().context("delete service")?;
    Ok(())
}

/// `forge-server service start` — start an already-installed service.
pub fn start() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("connect to service manager")?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::START)
        .context("open service")?;
    service.start::<&str>(&[]).context("start service")?;
    Ok(())
}

/// `forge-server service stop` — stop a running service. No-op if already
/// stopped.
pub fn stop() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("connect to service manager")?;
    let service = manager
        .open_service(
            SERVICE_NAME,
            ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
        )
        .context("open service")?;
    let _ = service.stop();
    Ok(())
}
