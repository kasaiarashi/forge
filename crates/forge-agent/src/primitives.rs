// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Builtin primitive dispatcher. Keyed on the `uses: @builtin/<name>`
//! string, each primitive takes a `with:` map and produces a shell command
//! to execute plus a set of output values. The actual command execution
//! still goes through the normal streaming shell runner in `runner.rs`.

use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::path::PathBuf;

/// Result of resolving a primitive: a shell command to run (empty = no
/// external process; outputs populated directly) plus outputs to thread
/// into subsequent expression-context lookups.
pub struct PrimitiveOutcome {
    pub command: Option<String>,
    pub outputs: HashMap<String, String>,
}

/// Dispatch a `@builtin/<name>` call. Unknown primitives return a clear
/// error so a composite referencing a not-yet-implemented primitive
/// surfaces as a step failure rather than a silent no-op.
pub fn dispatch(name: &str, with: &IndexMap<String, String>) -> Result<PrimitiveOutcome> {
    match name {
        "@builtin/ue-discover" => ue_discover(with),
        "@builtin/run-uat" => run_uat(with),
        "@builtin/run-editor-cmd" => run_editor_cmd(with),
        "@builtin/unreal-pak" => unreal_pak(with),
        "@builtin/steamcmd" => steamcmd(with),
        "@builtin/buildpatchtool" => Err(anyhow!(
            "primitive @builtin/buildpatchtool is not yet implemented on this agent"
        )),
        "@builtin/parse-cook-log" | "@builtin/parse-automation-report" => {
            // No-op passthrough: the raw log already lands in the step
            // log. Structured parsing ships in a later agent version;
            // treat as success so composites don't stall.
            Ok(PrimitiveOutcome {
                command: None,
                outputs: HashMap::new(),
            })
        }
        "@builtin/upload-artifact" => Err(anyhow!(
            "primitive @builtin/upload-artifact is not yet implemented on this agent"
        )),
        other => Err(anyhow!("unknown primitive: {}", other)),
    }
}

/// Find a UE install on disk. Writes `engine_root`, `uat_path`, `editor_path`,
/// `unreal_pak_path` as outputs. Never shells out.
fn ue_discover(with: &IndexMap<String, String>) -> Result<PrimitiveOutcome> {
    let version = with
        .get("version")
        .cloned()
        .unwrap_or_else(|| "5.7".to_string());

    // Candidate roots. Explicit env override wins.
    let env_override = std::env::var(format!("UE_ROOT_{}", version.replace('.', "_"))).ok();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = env_override {
        candidates.push(PathBuf::from(p));
    }
    #[cfg(windows)]
    {
        candidates.push(PathBuf::from(format!(
            "C:/Program Files/Epic Games/UE_{}",
            version
        )));
        candidates.push(PathBuf::from(format!("D:/Epic/UE_{}", version)));
    }
    #[cfg(not(windows))]
    {
        candidates.push(PathBuf::from(format!("/opt/UnrealEngine/{}", version)));
        candidates.push(PathBuf::from(format!(
            "{}/UnrealEngine/{}",
            std::env::var("HOME").unwrap_or_default(),
            version
        )));
    }

    let engine_root = candidates.into_iter().find(|p| p.exists()).ok_or_else(|| {
        anyhow!(
            "no UE {} found on this agent; install it or set UE_ROOT_{}",
            version,
            version.replace('.', "_")
        )
    })?;

    let mut outputs = HashMap::new();
    outputs.insert(
        "engine_root".into(),
        engine_root.to_string_lossy().into_owned(),
    );
    let (uat, editor, unreal_pak) = if cfg!(windows) {
        (
            engine_root
                .join("Engine/Build/BatchFiles/RunUAT.bat")
                .to_string_lossy()
                .into_owned(),
            engine_root
                .join("Engine/Binaries/Win64/UnrealEditor-Cmd.exe")
                .to_string_lossy()
                .into_owned(),
            engine_root
                .join("Engine/Binaries/Win64/UnrealPak.exe")
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        (
            engine_root
                .join("Engine/Build/BatchFiles/RunUAT.sh")
                .to_string_lossy()
                .into_owned(),
            engine_root
                .join("Engine/Binaries/Linux/UnrealEditor-Cmd")
                .to_string_lossy()
                .into_owned(),
            engine_root
                .join("Engine/Binaries/Linux/UnrealPak")
                .to_string_lossy()
                .into_owned(),
        )
    };
    outputs.insert("uat_path".into(), uat);
    outputs.insert("editor_path".into(), editor);
    outputs.insert("unreal_pak_path".into(), unreal_pak);
    Ok(PrimitiveOutcome {
        command: None,
        outputs,
    })
}

fn run_uat(with: &IndexMap<String, String>) -> Result<PrimitiveOutcome> {
    let uat = with
        .get("uat")
        .ok_or_else(|| anyhow!("run-uat needs 'uat' input (path to RunUAT.bat/.sh)"))?;
    let args = with.get("args").cloned().unwrap_or_default();
    // UAT on Windows needs the .bat invoked via cmd /C inside the shell
    // command we generate; on Linux it's a plain shell script. Either
    // way the outer run loop already wraps in `sh -c` / `cmd /C`.
    let cmd = format!("\"{}\" {}", uat, args);
    Ok(PrimitiveOutcome {
        command: Some(cmd),
        outputs: HashMap::new(),
    })
}

fn run_editor_cmd(with: &IndexMap<String, String>) -> Result<PrimitiveOutcome> {
    let editor = with
        .get("editor")
        .ok_or_else(|| anyhow!("run-editor-cmd needs 'editor' input"))?;
    let project = with
        .get("project")
        .ok_or_else(|| anyhow!("run-editor-cmd needs 'project' input"))?;
    let cmds = with.get("cmds").cloned().unwrap_or_default();
    let extra = with.get("extra").cloned().unwrap_or_default();
    let cmd = format!(
        "\"{}\" \"{}\" -ExecCmds=\"{}\" -unattended -nopause -nosplash {}",
        editor, project, cmds, extra
    );
    Ok(PrimitiveOutcome {
        command: Some(cmd),
        outputs: HashMap::new(),
    })
}

fn unreal_pak(with: &IndexMap<String, String>) -> Result<PrimitiveOutcome> {
    let pak = with
        .get("unreal_pak")
        .ok_or_else(|| anyhow!("unreal-pak needs 'unreal_pak' path"))?;
    let cooked = with
        .get("cooked_dir")
        .ok_or_else(|| anyhow!("unreal-pak needs 'cooked_dir'"))?;
    let output = with
        .get("output")
        .cloned()
        .unwrap_or_else(|| "./default.pak".to_string());
    let key = with.get("encryption_key").cloned().unwrap_or_default();
    // Encryption key must be empty or come from a secret. The server-side
    // secret expansion has already substituted `${{ secrets.* }}` before
    // the agent sees `with`; if the user hand-wrote a literal key, refuse.
    if !key.is_empty() && key.len() < 16 {
        return Err(anyhow!(
            "unreal-pak: encryption_key looks like an inline literal; pass it via ${{{{ secrets.X }}}} instead"
        ));
    }
    let extra = if key.is_empty() {
        String::new()
    } else {
        format!("-encryptionkey={}", key)
    };
    let cmd = format!(
        "\"{}\" \"{}\" -create=\"{}\" {}",
        pak, output, cooked, extra
    );
    Ok(PrimitiveOutcome {
        command: Some(cmd),
        outputs: HashMap::new(),
    })
}

fn steamcmd(with: &IndexMap<String, String>) -> Result<PrimitiveOutcome> {
    let app_id = with
        .get("app_id")
        .ok_or_else(|| anyhow!("steamcmd needs app_id"))?;
    let depot_id = with
        .get("depot_id")
        .ok_or_else(|| anyhow!("steamcmd needs depot_id"))?;
    let content_root = with
        .get("content_root")
        .ok_or_else(|| anyhow!("steamcmd needs content_root"))?;
    // steamcmd credentials must come from env (FORGE_STEAM_USER,
    // FORGE_STEAM_PASS). The user wires them in via workflow env referencing
    // secrets. We don't accept creds in `with:` — refusing keeps them out
    // of YAML auditing tools and off the process argv.
    let cmd = format!(
        "steamcmd +login $FORGE_STEAM_USER $FORGE_STEAM_PASS \
         +run_app_build_http {{depot {} app {} content \"{}\" }} +quit",
        depot_id, app_id, content_root
    );
    Ok(PrimitiveOutcome {
        command: Some(cmd),
        outputs: HashMap::new(),
    })
}
