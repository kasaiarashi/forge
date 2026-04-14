// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Shell selection for workflow `run:` steps.
//!
//! Centralises the `shell:` → `(program, flag)` mapping so both the
//! in-process engine and the distributed agent pick the same interpreter
//! for the same YAML. Keeping this in one place also means the set of
//! supported shells is obvious when an operator asks "does Forge run my
//! PowerShell step?".

/// Resolve a user-supplied `shell:` spec (or `None` for host default)
/// into the `(program, flag)` pair that invokes the one-liner. Unknown
/// names fall back to the host default with a warning — refusing to run
/// a long workflow because someone wrote `shell: fish` would be unkind.
pub fn resolve_shell(spec: Option<&str>) -> (&'static str, &'static str) {
    match spec.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("sh") => ("sh", "-c"),
        Some("bash") => ("bash", "-c"),
        Some("cmd") => ("cmd", "/C"),
        // `powershell` is Windows PowerShell 5.x; `pwsh` is PowerShell 7+.
        // `-Command` takes the script body on the same argv just like the
        // `-c` flag on POSIX shells.
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
