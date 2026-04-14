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

pub fn run(check_only: bool, force: bool, version: Option<String>) -> Result<()> {
    let current = parse_version(CURRENT_VERSION)?;

    // Pinned version → fetch that exact release tag. No tag → latest.
    // GitHub accepts either "v0.1.0" or "0.1.0" as the tag, but the
    // releases API path expects the exact tag as published, so we try
    // the user's input verbatim first and retry with a "v" prefix when
    // it's missing.
    let api_url = match &version {
        Some(tag) => format!(
            "https://api.github.com/repos/{}/releases/tags/{}",
            GITHUB_REPO, tag
        ),
        None => format!(
            "https://api.github.com/repos/{}/releases/latest",
            GITHUB_REPO
        ),
    };
    let body = match (http_get_string(&api_url), &version) {
        (Ok(b), _) => b,
        (Err(_), Some(tag)) if !tag.starts_with('v') => {
            let retry = format!(
                "https://api.github.com/repos/{}/releases/tags/v{}",
                GITHUB_REPO, tag
            );
            http_get_string(&retry).with_context(|| {
                format!("No release found for tag '{}' (also tried 'v{}')", tag, tag)
            })?
        }
        (Err(e), _) => return Err(e),
    };
    let release: Release =
        serde_json::from_str(&body).context("Failed to parse release JSON")?;

    let latest = parse_version(&release.tag_name)?;
    // When pinning a version, treat "not equal" as needing an update so
    // downgrades work too. --force still bypasses the version check.
    let needs_update = if version.is_some() {
        latest != current || force
    } else {
        latest > current || force
    };

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

    // std::fs::copy preserves the source file's permissions on Unix. curl
    // writes the tmp with a default 0644, so the copied binary lands
    // non-executable — forge-web then won't launch after an update. Mark
    // the tmp executable before copy so the destination inherits 0755.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&tmp_path)
            .with_context(|| format!("stat {}", tmp_path.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmp_path, perms)
            .with_context(|| format!("chmod {}", tmp_path.display()))?;
    }

    // Unlink the target before copying. A running forge-web holds the
    // file busy; std::fs::copy opens with O_TRUNC and fails with ETXTBSY
    // ("Text file busy"). On Linux, unlinking a busy binary is legal —
    // the kernel keeps the old inode alive for the running process and
    // the new copy lands on a fresh inode.
    let _ = std::fs::remove_file(target_path);

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
    // Remove any stale tmp from a prior run. A previous `sudo forge-server
    // update` leaves /tmp/forge-update-* owned by root with 0644; the next
    // run (possibly under a different uid) can't truncate it and curl bails
    // with exit 23 "Failure writing output to destination".
    let _ = std::fs::remove_file(dest);
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
