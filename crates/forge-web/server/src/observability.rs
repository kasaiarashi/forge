// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Logging setup for forge-web.
//!
//! Mirrors `forge-server::observability::init` — same three-sink layout
//! (stdout + rolling app file + rolling audit file), same config shape.
//! Kept as a small duplicate rather than sharing a crate because the
//! list of connective types between web and server is already small and
//! adding a shared-utility crate for one module isn't worth the churn.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::filter::{EnvFilter, Targets};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

use crate::config::LoggingConfig;

pub struct LogGuards {
    _app: Option<WorkerGuard>,
    _audit: Option<WorkerGuard>,
}

pub fn init(logging: &LoggingConfig, log_dir: Option<&Path>) -> LogGuards {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&logging.level));

    let mut app_guard: Option<WorkerGuard> = None;
    let mut audit_guard: Option<WorkerGuard> = None;

    let want_stdout = logging.stdout || log_dir.is_none();
    let stdout_layer: Option<Box<dyn Layer<_> + Send + Sync + 'static>> = if want_stdout {
        Some(match logging.format.as_str() {
            "json" => fmt::layer()
                .json()
                .with_writer(std::io::stdout)
                .with_filter(audit_off_when_file(log_dir.is_some()))
                .boxed(),
            _ => fmt::layer()
                .with_writer(std::io::stdout)
                .with_filter(audit_off_when_file(log_dir.is_some()))
                .boxed(),
        })
    } else {
        None
    };

    let app_layer: Option<Box<dyn Layer<_> + Send + Sync + 'static>> = match log_dir {
        Some(dir) => {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!(
                    "forge-web: failed to create log dir {:?}: {e}; falling back to stdout",
                    dir
                );
                None
            } else {
                let appender = rolling::daily(dir, "forge-web.log");
                let (nb, guard) = tracing_appender::non_blocking(appender);
                app_guard = Some(guard);
                let layer = match logging.format.as_str() {
                    "json" => fmt::layer()
                        .json()
                        .with_writer(nb)
                        .with_filter(
                            Targets::new()
                                .with_target("audit", tracing::Level::ERROR)
                                .with_default(tracing::Level::TRACE),
                        )
                        .boxed(),
                    _ => fmt::layer()
                        .with_ansi(false)
                        .with_writer(nb)
                        .with_filter(
                            Targets::new()
                                .with_target("audit", tracing::Level::ERROR)
                                .with_default(tracing::Level::TRACE),
                        )
                        .boxed(),
                };
                Some(layer)
            }
        }
        None => None,
    };

    let audit_layer: Option<Box<dyn Layer<_> + Send + Sync + 'static>> = match log_dir {
        Some(dir) => {
            let appender = rolling::daily(dir, "audit.log");
            let (nb, guard) = tracing_appender::non_blocking(appender);
            audit_guard = Some(guard);
            let layer = fmt::layer()
                .json()
                .with_writer(nb)
                .with_filter(Targets::new().with_target("audit", tracing::Level::INFO))
                .boxed();
            Some(layer)
        }
        None => None,
    };

    let result = tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(app_layer)
        .with(audit_layer)
        .try_init();
    if let Err(e) = result {
        eprintln!("forge-web: tracing init: {e}");
    }

    LogGuards {
        _app: app_guard,
        _audit: audit_guard,
    }
}

fn audit_off_when_file(have_file: bool) -> Targets {
    if have_file {
        Targets::new()
            .with_target("audit", tracing::Level::ERROR)
            .with_default(tracing::Level::TRACE)
    } else {
        Targets::new().with_default(tracing::Level::TRACE)
    }
}

/// Emit a structured audit event from forge-web. Same contract as the
/// forge-server `audit!` macro: pin the target to `"audit"` so the
/// dedicated sink picks it up.
#[macro_export]
macro_rules! audit {
    ($($field:tt)*) => {
        ::tracing::info!(target: "audit", $($field)*)
    };
}
