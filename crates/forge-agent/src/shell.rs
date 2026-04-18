// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Shell selection for `run:` steps.
//!
//! Mirror of `forge-server::services::actions::shell::resolve_shell` so
//! a workflow that runs on the in-process engine picks the same
//! interpreter on a remote agent. Kept as a small duplicate instead of a
//! shared crate because the only connective tissue between the two is
//! this table and sharing would drag tracing + serde into forge-core.

pub fn resolve_shell(spec: Option<&str>) -> (&'static str, &'static str) {
    match spec.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("sh") => ("sh", "-c"),
        Some("bash") => ("bash", "-c"),
        Some("cmd") => ("cmd", "/C"),
        Some("powershell") => ("powershell", "-Command"),
        Some("pwsh") => ("pwsh", "-Command"),
        Some(other) => {
            tracing::warn!(
                shell = other,
                "unknown shell spec; falling back to host default"
            );
            host_default()
        }
        None => host_default(),
    }
}

fn host_default() -> (&'static str, &'static str) {
    if cfg!(target_os = "windows") {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    }
}
