// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Logging, audit, and request-tracing wiring.
//!
//! Three sinks may be active at once:
//!
//! * **stdout** — text or JSON, gated by `LoggingSection::stdout` (always
//!   on when no file sink is configured).
//! * **application log file** — rolling daily, captures everything at
//!   `logging.level`, minus the `audit` target which is routed away.
//! * **audit log file** — rolling daily, captures only events whose
//!   target is `audit`. Always emitted at `info` and above, regardless of
//!   the app-log level, because audit is the whole point of the file.
//!
//! The [`audit!`] macro below is the only way we emit audit events;
//! sprinkling `tracing::info!(target: "audit", ...)` by hand would work
//! but the macro keeps the shape (outcome, actor, action) consistent.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::filter::{EnvFilter, Targets};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

use crate::config::LoggingSection;

/// Handles that must stay alive for background writer threads to flush.
/// Dropping these at shutdown is what causes the final log lines to be
/// written — losing the guards at the wrong moment silently drops logs.
pub struct LogGuards {
    _app: Option<WorkerGuard>,
    _audit: Option<WorkerGuard>,
}

/// Initialise the global tracing subscriber. Returns [`LogGuards`] that
/// **must** be held for the process lifetime. Idempotent-ish: on the
/// second call `try_init` fails softly and we return empty guards, which
/// is useful for tests that spin up the server in-process.
pub fn init(logging: &LoggingSection, log_dir: Option<&Path>) -> LogGuards {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&logging.level));

    let mut app_guard: Option<WorkerGuard> = None;
    let mut audit_guard: Option<WorkerGuard> = None;

    // stdout layer — on when (a) operator opted in OR (b) no file sink.
    let want_stdout = logging.stdout || log_dir.is_none();
    let stdout_layer: Option<Box<dyn Layer<_> + Send + Sync + 'static>> = if want_stdout {
        Some(match logging.format.as_str() {
            "json" => fmt::layer()
                .json()
                .with_writer(std::io::stdout)
                // Keep audit events off stdout when a file sink is also
                // active — duplicate audit lines are noisy and misleading.
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

    // Application file layer (non-audit only).
    let app_layer: Option<Box<dyn Layer<_> + Send + Sync + 'static>> = match log_dir {
        Some(dir) => {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!(
                    "forge-server: failed to create log dir {:?}: {e}; falling back to stdout",
                    dir
                );
                None
            } else {
                let appender = rolling::daily(dir, "forge-server.log");
                let (nb, guard) = tracing_appender::non_blocking(appender);
                app_guard = Some(guard);
                // `with_ansi(false)` — nobody wants escape codes in a log file.
                let layer = match logging.format.as_str() {
                    "json" => fmt::layer()
                        .json()
                        .with_writer(nb)
                        .with_filter(Targets::new().with_target("audit", tracing::Level::ERROR).with_default(tracing::Level::TRACE))
                        .boxed(),
                    _ => fmt::layer()
                        .with_ansi(false)
                        .with_writer(nb)
                        .with_filter(Targets::new().with_target("audit", tracing::Level::ERROR).with_default(tracing::Level::TRACE))
                        .boxed(),
                };
                Some(layer)
            }
        }
        None => None,
    };

    // Dedicated audit file layer.
    let audit_layer: Option<Box<dyn Layer<_> + Send + Sync + 'static>> = match log_dir {
        Some(dir) => {
            let appender = rolling::daily(dir, "audit.log");
            let (nb, guard) = tracing_appender::non_blocking(appender);
            audit_guard = Some(guard);
            let layer = fmt::layer()
                .json()
                .with_writer(nb)
                .with_filter(
                    // Only the audit target reaches this sink. No matter
                    // what RUST_LOG says, audit is always recorded.
                    Targets::new().with_target("audit", tracing::Level::INFO),
                )
                .boxed();
            Some(layer)
        }
        None => None,
    };

    // `Option<Layer>` is itself a `Layer`, so optional sinks chain cleanly
    // without matching on every presence combination.
    let result = tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(app_layer)
        .with(audit_layer)
        .try_init();
    if let Err(e) = result {
        // A second init from the same process is fine (tests); anything
        // else is an operator config mistake we'd rather surface.
        eprintln!("forge-server: tracing init: {e}");
    }

    LogGuards {
        _app: app_guard,
        _audit: audit_guard,
    }
}

/// Filter that silences the `audit` target on a layer when the dedicated
/// audit file sink is active — otherwise stdout would carry the audit
/// line and the file would too, and downstream log scrapers would
/// double-count.
fn audit_off_when_file(have_file: bool) -> Targets {
    if have_file {
        Targets::new()
            .with_target("audit", tracing::Level::ERROR)
            .with_default(tracing::Level::TRACE)
    } else {
        Targets::new().with_default(tracing::Level::TRACE)
    }
}

/// Emit a structured audit event.
///
/// Every call records at minimum `outcome` and `action`; the caller adds
/// whatever context makes the event useful (actor, repo, target id,
/// reason, source IP, …). The macro exists to pin the tracing target to
/// `"audit"` so the dedicated sink picks it up.
///
/// Usage:
/// ```ignore
/// audit!(action = "login", outcome = "success", actor = %user, ip = %ip);
/// audit!(action = "repo.delete", outcome = "denied", actor = %user, repo = %name, reason = "not admin");
/// ```
#[macro_export]
macro_rules! audit {
    ($($field:tt)*) => {
        ::tracing::info!(target: "audit", $($field)*)
    };
}
