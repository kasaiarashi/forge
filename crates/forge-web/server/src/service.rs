// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Windows service integration for forge-web.
//!
//! Twin of `forge-server/src/service.rs`. The only meaningful differences
//! are the SCM identifiers and the call into [`crate::serve_inner`]
//! instead of `forge-server`'s. See the parent module's doc comment for
//! the design rationale.

#![cfg(windows)]

use anyhow::{anyhow, Context, Result};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl,
        ServiceExitCode, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

use crate::config::Config;

pub const SERVICE_NAME: &str = "ForgeWeb";
pub const DISPLAY_NAME: &str = "Forge VCS Web UI";
pub const DESCRIPTION: &str =
    "Forge VCS web UI — browser frontend for the gRPC server.";

const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

static SERVICE_PAYLOAD: OnceLock<ServicePayload> = OnceLock::new();

pub struct ServicePayload {
    pub config: Config,
}

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
        eprintln!("forge-web service exited with error: {e:#}");
    }
}

fn run_service() -> Result<()> {
    let payload = SERVICE_PAYLOAD
        .get()
        .ok_or_else(|| anyhow!("service payload missing — run_under_scm not called"))?;

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

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    *stop_tx_holder.lock().unwrap() = Some(stop_tx);

    set_status(
        &status_handle,
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        Duration::default(),
    )?;

    let serve_result = build_runtime()?.block_on(async move {
        crate::serve_inner(payload.config.clone(), async move {
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
        .thread_name("forge-web-svc")
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
        launch_arguments: vec![
            OsString::from("--config"),
            OsString::from(config_path),
            OsString::from("--as-service"),
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

pub fn uninstall() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("connect to service manager")?;

    let service = match manager.open_service(
        SERVICE_NAME,
        ServiceAccess::DELETE | ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
    ) {
        Ok(s) => s,
        Err(windows_service::Error::Winapi(io_err))
            if io_err.raw_os_error() == Some(1060) =>
        {
            return Ok(());
        }
        Err(e) => return Err(anyhow!("open service: {e}")),
    };

    let _ = service.stop();
    std::thread::sleep(Duration::from_secs(2));
    service.delete().context("delete service")?;
    Ok(())
}

pub fn start() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("connect to service manager")?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::START)
        .context("open service")?;
    service.start::<&str>(&[]).context("start service")?;
    Ok(())
}

pub fn stop() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("connect to service manager")?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::STOP | ServiceAccess::QUERY_STATUS)
        .context("open service")?;
    let _ = service.stop();
    Ok(())
}
