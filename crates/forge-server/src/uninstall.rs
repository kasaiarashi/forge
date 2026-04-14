//! Linux uninstall flow. Mirrors `installers/linux/install.sh` — every
//! path this function touches is something the installer creates, and
//! nothing outside that set. Refuses to run unless invoked as root
//! because every step (systemd unit removal, /usr/local/bin writes,
//! userdel) needs it; bailing early with a clear message is friendlier
//! than surfacing ten individual permission errors.

use anyhow::{bail, Context, Result};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Default install locations — match the defaults in install.sh. We
/// could read them out of /etc/forge/forge-server.toml but the toml
/// itself is one of the files we're about to delete, and the installer
/// never writes its own layout back there. Hardcoding keeps the two
/// scripts trivially in sync.
const DEFAULT_PREFIX: &str = "/usr/local";
const DEFAULT_CONFIG_DIR: &str = "/etc/forge";
const DEFAULT_DATA_DIR: &str = "/var/lib/forge";

pub fn run(purge: bool, yes: bool) -> Result<()> {
    if !is_root() {
        bail!("forge-server uninstall must be run as root (try: sudo forge-server uninstall)");
    }

    let prefix = std::env::var("PREFIX").unwrap_or_else(|_| DEFAULT_PREFIX.into());
    let config_dir =
        std::env::var("CONFIG_DIR").unwrap_or_else(|_| DEFAULT_CONFIG_DIR.into());
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| DEFAULT_DATA_DIR.into());

    println!("This will remove the following:");
    println!("  Binaries:       {prefix}/bin/forge-server, {prefix}/bin/forge-web");
    println!("  Web UI:         {prefix}/share/forge/");
    println!("  Config dir:     {config_dir}/");
    println!("  systemd units:  /etc/systemd/system/forge-server.service, forge-web.service");
    println!("  Profile hook:   /etc/profile.d/forge.sh");
    println!("  System user:    forge (and the 'forge' group)");
    if purge {
        println!("  Data dir:       {data_dir}/  (--purge: DB + objects + certs WILL be deleted)");
    } else {
        println!("  Data dir:       {data_dir}/  (kept — re-run with --purge to delete)");
    }
    println!();

    if !yes && !confirm("Proceed with uninstall? [y/N]: ")? {
        println!("Aborted.");
        return Ok(());
    }

    // Order matters: stop services before deleting their binaries or the
    // systemd unit files, so systemd has a chance to SIGTERM cleanly.
    if systemd_active() {
        for unit in ["forge-web.service", "forge-server.service"] {
            if unit_exists(unit) {
                println!("Stopping {unit}...");
                let _ = run_quiet("systemctl", &["stop", unit]);
                let _ = run_quiet("systemctl", &["disable", unit]);
            }
        }
    }

    remove_file_if_exists("/etc/systemd/system/forge-server.service");
    remove_file_if_exists("/etc/systemd/system/forge-web.service");
    if systemd_active() {
        let _ = run_quiet("systemctl", &["daemon-reload"]);
    }

    remove_file_if_exists("/etc/profile.d/forge.sh");

    remove_file_if_exists(&format!("{prefix}/bin/forge-server"));
    remove_file_if_exists(&format!("{prefix}/bin/forge-web"));
    remove_dir_if_exists(&format!("{prefix}/share/forge"));
    remove_dir_if_exists(&config_dir);

    if purge {
        remove_dir_if_exists(&data_dir);
    } else {
        println!("Preserving {data_dir}/ (run with --purge to delete).");
    }

    // User/group removal last — they may still own files at this point
    // if an errored earlier step left stragglers; userdel with --force
    // removes the user even when still logged-in sessions exist on some
    // distros, but we fall back to logging a warning if it fails rather
    // than rolling back partial uninstall.
    if getent_exists("passwd", "forge") {
        println!("Removing 'forge' system user...");
        if let Err(e) = run_quiet("userdel", &["forge"]) {
            eprintln!("  Warning: userdel failed: {e}");
            eprintln!("  Remove manually: sudo userdel forge");
        }
    }
    if getent_exists("group", "forge") {
        // `userdel` usually drops the primary group too; only call
        // groupdel as a fallback when it stuck around (e.g. because
        // other users still list 'forge' as a secondary group).
        if let Err(e) = run_quiet("groupdel", &["forge"]) {
            // Not fatal — groupdel refuses to remove a group that still
            // has members, which is the correct safety behavior.
            eprintln!("  Note: groupdel forge skipped ({e}). Other users may still be members.");
        }
    }

    println!();
    println!("forge-server uninstalled.");
    if !purge {
        println!("Data preserved at {data_dir} — delete manually when you're sure.");
    }
    // NOTE: the running forge-server binary itself still exists until
    // this process exits. self-delete isn't portable and isn't worth
    // the complexity; `rm` already ran against $prefix/bin/forge-server
    // above so the file is gone from disk — this process is the last
    // reference and cleans up on exit.
    Ok(())
}

fn is_root() -> bool {
    // nix-free uid check. getuid is always 0 for root; no crate dep needed.
    // SAFETY: getuid is always-safe — no preconditions, never fails.
    unsafe { libc_getuid() == 0 }
}

// Hand-rolled libc binding so this module doesn't drag in the `libc`
// crate just to call one function. getuid's ABI is stable across every
// Linux libc (glibc, musl, bionic).
extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("Failed to read confirmation")?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn systemd_active() -> bool {
    Path::new("/run/systemd/system").is_dir()
}

fn unit_exists(unit: &str) -> bool {
    Path::new(&format!("/etc/systemd/system/{unit}")).exists()
}

fn getent_exists(db: &str, name: &str) -> bool {
    Command::new("getent")
        .args([db, name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_quiet(cmd: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .with_context(|| format!("failed to run {cmd}"))?;
    if !status.success() {
        bail!("{cmd} {} exited {}", args.join(" "), status);
    }
    Ok(())
}

fn remove_file_if_exists(path: &str) {
    let p: PathBuf = path.into();
    if p.exists() {
        println!("Removing {path}");
        if let Err(e) = std::fs::remove_file(&p) {
            eprintln!("  Warning: could not remove {path}: {e}");
        }
    }
}

fn remove_dir_if_exists(path: &str) {
    let p: PathBuf = path.into();
    if p.exists() {
        println!("Removing {path}");
        if let Err(e) = std::fs::remove_dir_all(&p) {
            eprintln!("  Warning: could not remove {path}: {e}");
        }
    }
}
