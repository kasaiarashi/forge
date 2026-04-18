use anyhow::{bail, Context, Result};
use std::process::Command;

const GITHUB_REPO: &str = "kasaiarashi/forge";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
    html_url: String,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

fn asset_name() -> &'static str {
    #[cfg(target_os = "windows")]
    { "forge-windows-x64.exe" }
    #[cfg(target_os = "linux")]
    { "forge-linux-x64" }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    { "forge-macos-arm64" }
    #[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
    { "forge-macos-x64" }
}

pub fn run(check_only: bool, force: bool, json: bool) -> Result<()> {
    let current = parse_version(CURRENT_VERSION)?;

    let api_url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );
    let body = http_get_string(&api_url)?;
    let release: Release =
        serde_json::from_str(&body).context("Failed to parse release JSON")?;

    let latest = parse_version(&release.tag_name)?;
    let needs_update = latest > current || force;

    if json {
        let obj = serde_json::json!({
            "current_version": format_version(&current),
            "latest_version": format_version(&latest),
            "update_available": latest > current,
            "release_url": release.html_url,
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
        if check_only || !needs_update {
            return Ok(());
        }
    } else if !needs_update {
        println!("forge is up to date (v{})", format_version(&current));
        return Ok(());
    } else {
        if force && latest <= current {
            println!(
                "Forcing re-download of v{} (same version)",
                format_version(&latest)
            );
        } else {
            println!(
                "Update available: v{} -> v{}",
                format_version(&current),
                format_version(&latest)
            );
        }
        if check_only {
            println!("Release: {}", release.html_url);
            println!("\nRun `forge update` to install.");
            return Ok(());
        }
    }

    let expected = asset_name();
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == expected)
        .with_context(|| format!("No release asset for this platform (expected: {})", expected))?;

    if !json {
        println!(
            "Downloading {} ({:.1} MB)...",
            asset.name,
            asset.size as f64 / 1_048_576.0
        );
    }

    // Resolve the running binary up-front so every step of the update
    // refers to the same path. `std::env::current_exe()` returns the
    // canonical image path of this process — that's the file
    // `self_replace` will overwrite. Print it so users who installed
    // via multiple channels (InnoSetup + cargo install + scoop, etc.)
    // can see which copy is being touched.
    let exe_path = std::env::current_exe().context("determine current executable path")?;
    if !json {
        println!("Replacing: {}", exe_path.display());
    }

    let tmp_path = std::env::temp_dir().join(format!("forge-update-{}", expected));
    http_download_file(&asset.browser_download_url, &tmp_path)?;

    self_replace::self_replace(&tmp_path)
        .context("Failed to replace binary. Try running with administrator privileges.")?;

    let _ = std::fs::remove_file(&tmp_path);

    // Verify the replacement actually took effect. `self_replace` has
    // returned Ok in the past on Windows when a scheduled-for-delete
    // fallback kicked in or when the target path was a PATH-shadowed
    // duplicate — both leave the old binary on disk and produce a
    // confusing "Updated forge to vX" message followed by `forge
    // version` still showing the old number. Spawn the freshly-replaced
    // exe with `--version` and confirm it reports the new version
    // before claiming success.
    let verified = verify_installed_version(&exe_path, &latest);
    match &verified {
        Ok(installed) if installed == &latest => {
            if json {
                let obj = serde_json::json!({
                    "status": "updated",
                    "version": format_version(&latest),
                    "path": exe_path.display().to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&obj)?);
            } else {
                println!(
                    "Updated forge to v{} (verified at {})",
                    format_version(&latest),
                    exe_path.display()
                );
            }
            Ok(())
        }
        Ok(installed) => {
            let hint = path_shadowing_hint(&exe_path);
            bail!(
                "Replacement ran but `{exe} --version` still reports v{} (expected v{}). \
                 The binary at {exe} may be a PATH-shadowed duplicate or a shim.{hint}",
                format_version(installed),
                format_version(&latest),
                exe = exe_path.display(),
                hint = hint,
            );
        }
        Err(e) => {
            bail!(
                "Replacement ran but failed to verify installed version via \
                 `{} --version`: {e}. Check the binary at that path before relying \
                 on the update.",
                exe_path.display()
            );
        }
    }
}

/// Run `<exe> --version` and parse the installed version string. Used
/// to confirm a `self_replace` call actually took effect rather than
/// silently fell through to a no-op. Any non-zero exit, unparseable
/// output, or spawn failure surfaces as `Err` so the caller prints a
/// diagnostic instead of a false-positive success message.
fn verify_installed_version(exe: &std::path::Path, expected: &SemVer) -> Result<SemVer> {
    let out = Command::new(exe)
        .arg("--version")
        .output()
        .with_context(|| format!("spawn `{} --version`", exe.display()))?;
    if !out.status.success() {
        bail!(
            "`{} --version` exited with status {}",
            exe.display(),
            out.status
        );
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // clap's default `--version` output is `"<binary> <version>\n"` —
    // e.g. `"forge 0.2.0"`. Take the first whitespace-separated token
    // that parses as semver.
    for token in stdout.split_whitespace() {
        if let Ok(v) = parse_version(token) {
            let _ = expected; // retained for future "suggest a re-run" diagnostics.
            return Ok(v);
        }
    }
    bail!(
        "could not parse a version number from `{} --version` output: {stdout:?}",
        exe.display()
    );
}

/// Best-effort diagnostic: if multiple `forge` binaries sit on PATH,
/// an update that rewrites one while the shell resolves to a different
/// copy on the next invocation is a common source of "update reported
/// success but version didn't change" reports. Return a short hint
/// listing every `forge` discoverable on PATH; empty string when only
/// one (or none) is found.
fn path_shadowing_hint(replaced: &std::path::Path) -> String {
    let Some(path_env) = std::env::var_os("PATH") else {
        return String::new();
    };
    let exe_name = if cfg!(windows) { "forge.exe" } else { "forge" };
    let mut hits = Vec::new();
    for dir in std::env::split_paths(&path_env) {
        let candidate = dir.join(exe_name);
        if candidate.is_file() {
            // Canonicalise both sides so case-insensitive Windows paths
            // and relative PATH entries compare correctly.
            let canon = candidate
                .canonicalize()
                .unwrap_or_else(|_| candidate.clone());
            if !hits.iter().any(|p| p == &canon) {
                hits.push(canon);
            }
        }
    }
    if hits.len() <= 1 {
        return String::new();
    }
    let replaced_canon = replaced
        .canonicalize()
        .unwrap_or_else(|_| replaced.to_path_buf());
    let mut msg = String::from("\n\nMultiple forge binaries found on PATH:");
    for p in &hits {
        let marker = if p == &replaced_canon { " (updated)" } else { "" };
        msg.push_str(&format!("\n  - {}{}", p.display(), marker));
    }
    msg.push_str(
        "\n\nYour shell may resolve `forge` to a copy that wasn't updated. \
         Either remove the other entries from PATH or reinstall via the \
         same channel you originally used.",
    );
    msg
}

// Simple semver tuple: (major, minor, patch)
type SemVer = (u64, u64, u64);

fn parse_version(s: &str) -> Result<SemVer> {
    let s = s.strip_prefix('v').unwrap_or(s);
    // Strip pre-release suffix (e.g. "0.1.0-rc.1" -> "0.1.0")
    let s = s.split('-').next().unwrap_or(s);
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        bail!("Invalid version: {}", s);
    }
    Ok((
        parts[0].parse().context("Invalid major version")?,
        parts[1].parse().context("Invalid minor version")?,
        parts[2].parse().context("Invalid patch version")?,
    ))
}

fn format_version(v: &SemVer) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

/// Fetch a URL as a string using platform tools.
fn http_get_string(url: &str) -> Result<String> {
    #[cfg(target_os = "windows")]
    {
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
                     (Invoke-WebRequest -Uri '{}' -UseBasicParsing -Headers @{{'User-Agent'='forge-cli/{}'}}).Content",
                    url, CURRENT_VERSION
                ),
            ])
            .output()
            .context("Failed to run powershell")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("HTTP request failed: {}", stderr);
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new("curl")
            .args([
                "-fsSL",
                "-H",
                &format!("User-Agent: forge-cli/{}", CURRENT_VERSION),
                url,
            ])
            .output()
            .context("Failed to run curl")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("HTTP request failed: {}", stderr);
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Download a URL to a file using platform tools.
fn http_download_file(url: &str, dest: &std::path::Path) -> Result<()> {
    let dest_str = dest.to_string_lossy();

    #[cfg(target_os = "windows")]
    {
        let status = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
                     Invoke-WebRequest -Uri '{}' -OutFile '{}' -UseBasicParsing",
                    url, dest_str
                ),
            ])
            .status()
            .context("Failed to run powershell")?;

        if !status.success() {
            bail!("Download failed");
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let status = Command::new("curl")
            .args(["-fsSL", "-o", &dest_str, url])
            .status()
            .context("Failed to run curl")?;

        if !status.success() {
            bail!("Download failed");
        }
    }

    Ok(())
}
