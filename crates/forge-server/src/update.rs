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

fn server_asset_name() -> &'static str {
    #[cfg(target_os = "windows")]
    { "forge-server-windows-x64.exe" }
    #[cfg(target_os = "linux")]
    { "forge-server-linux-x64" }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    { "forge-server-macos-arm64" }
    #[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
    { "forge-server-macos-x64" }
}

fn web_asset_name() -> &'static str {
    #[cfg(target_os = "windows")]
    { "forge-web-windows-x64.exe" }
    #[cfg(target_os = "linux")]
    { "forge-web-linux-x64" }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    { "forge-web-macos-arm64" }
    #[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
    { "forge-web-macos-x64" }
}

pub fn run(check_only: bool, force: bool) -> Result<()> {
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

    if !needs_update {
        println!("forge-server is up to date (v{})", format_version(&current));
        return Ok(());
    }

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
        println!("\nRun `forge-server update` to install.");
        return Ok(());
    }

    // Update forge-server (self-replace)
    download_and_self_replace(server_asset_name(), &release)?;

    // Also update forge-web if it lives next to forge-server
    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe.parent().unwrap();
    let web_exe = if cfg!(target_os = "windows") {
        exe_dir.join("forge-web.exe")
    } else {
        exe_dir.join("forge-web")
    };

    if web_exe.exists() {
        download_and_replace_file(web_asset_name(), &release, &web_exe)?;
    }

    println!("Updated to v{}", format_version(&latest));
    println!("Restart the server to use the new version.");

    Ok(())
}

fn download_and_self_replace(asset_name: &str, release: &Release) -> Result<()> {
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .with_context(|| format!("No release asset: {}", asset_name))?;

    println!(
        "Downloading {} ({:.1} MB)...",
        asset.name,
        asset.size as f64 / 1_048_576.0
    );

    let tmp_path = std::env::temp_dir().join(format!("forge-update-{}", asset_name));
    http_download_file(&asset.browser_download_url, &tmp_path)?;

    self_replace::self_replace(&tmp_path)
        .context("Failed to replace binary. Try running with administrator privileges.")?;

    let _ = std::fs::remove_file(&tmp_path);
    Ok(())
}

fn download_and_replace_file(
    asset_name: &str,
    release: &Release,
    target_path: &std::path::Path,
) -> Result<()> {
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .with_context(|| format!("No release asset: {}", asset_name))?;

    println!(
        "Downloading {} ({:.1} MB)...",
        asset.name,
        asset.size as f64 / 1_048_576.0
    );

    let tmp_path = std::env::temp_dir().join(format!("forge-update-{}", asset_name));
    http_download_file(&asset.browser_download_url, &tmp_path)?;

    std::fs::copy(&tmp_path, target_path)
        .with_context(|| format!("Failed to replace {}", target_path.display()))?;

    let _ = std::fs::remove_file(&tmp_path);
    Ok(())
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
                     (Invoke-WebRequest -Uri '{}' -UseBasicParsing -Headers @{{'User-Agent'='forge-server/{}'}}).Content",
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
                &format!("User-Agent: forge-server/{}", CURRENT_VERSION),
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
