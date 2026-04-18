// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Three-tier credential storage for the `forge` CLI.
//!
//! Lookup order, highest priority first:
//!
//! 1. **`FORGE_TOKEN` env var** — always wins for CI scenarios. Optionally
//!    paired with `FORGE_USER` and `FORGE_SERVER_URL`. CI bots set these and
//!    bypass everything else.
//! 2. **OS keychain** via the `keyring` crate. Service name `"forge-vcs"`,
//!    account name `"<server_url>"`. Cross-platform — Windows Credential
//!    Manager, macOS Keychain, libsecret on Linux. Falls through silently
//!    when the platform has no keychain available (e.g. headless box with
//!    no display server).
//! 3. **Plain JSON file** at `~/.forge/credentials` with mode 0600 on Unix.
//!    Schema: `{"<server_url>": {"user": "...", "token": "fpat_..."}}`.
//!    Same shape as `~/.docker/config.json` and `~/.aws/credentials`.
//!
//! Saves use the inverse: keychain if available, file otherwise. The env
//! var path is read-only — we never overwrite the user's `FORGE_TOKEN`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One server's stored credential.
///
/// `display_name` and `email` are populated from the WhoAmI response on
/// `forge login` so commands like `forge commit` can fall back to them when
/// the workspace's local `user.name` / `user.email` are unset. They're
/// `#[serde(default)]` so older credentials files without these fields keep
/// loading without re-login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub user: String,
    pub token: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub email: String,
}

const KEYRING_SERVICE: &str = "forge-vcs";
const ENV_TOKEN: &str = "FORGE_TOKEN";
const ENV_USER: &str = "FORGE_USER";
const ENV_SERVER: &str = "FORGE_SERVER_URL";

/// Load the credential for `server_url`. Returns `Ok(None)` if no credential
/// is configured anywhere — that's a normal state for an anonymous CLI run
/// against a public repo, not an error.
pub fn load(server_url: &str) -> Result<Option<Credential>> {
    // 1. Env var takes precedence — for CI.
    if let Some(cred) = env_credential(
        server_url,
        std::env::var(ENV_TOKEN).ok(),
        std::env::var(ENV_USER).ok(),
        std::env::var(ENV_SERVER).ok(),
    ) {
        return Ok(Some(cred));
    }

    // 2. Keychain.
    if let Some(cred) = load_from_keychain(server_url) {
        return Ok(Some(cred));
    }

    // 3. File.
    load_from_file(server_url)
}

/// Pure helper that decides whether the FORGE_TOKEN / FORGE_USER /
/// FORGE_SERVER_URL env vars should produce a credential for `server_url`.
/// Extracted from [`load`] so it's testable without mutating global env state.
fn env_credential(
    server_url: &str,
    token: Option<String>,
    user: Option<String>,
    env_server_filter: Option<String>,
) -> Option<Credential> {
    let token = token?;
    if token.is_empty() {
        return None;
    }
    // FORGE_SERVER_URL, when set, scopes the env-var token to a single server
    // so a CI box that talks to multiple forge servers doesn't leak its token
    // across them. When unset, the env var applies to every server.
    if let Some(filter) = env_server_filter {
        if filter != server_url {
            return None;
        }
    }
    Some(Credential {
        user: user.unwrap_or_default(),
        token,
        display_name: String::new(),
        email: String::new(),
    })
}

/// Save the credential for `server_url`. Tries the keychain first; falls
/// back to the JSON file if the keychain is unavailable.
pub fn save(server_url: &str, cred: &Credential) -> Result<&'static str> {
    if save_to_keychain(server_url, cred) {
        return Ok("OS keychain");
    }
    save_to_file(server_url, cred)?;
    Ok(file_path()?
        .to_str()
        .map(|_| "~/.forge/credentials")
        .unwrap_or("~/.forge/credentials"))
}

/// Forget the credential for `server_url`. Removes from both backends so a
/// stale entry in one doesn't shadow the other after a logout.
pub fn delete(server_url: &str) -> Result<()> {
    let _ = delete_from_keychain(server_url);
    delete_from_file(server_url)?;
    Ok(())
}

// ── Keychain backend ─────────────────────────────────────────────────────────

fn load_from_keychain(server_url: &str) -> Option<Credential> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, server_url).ok()?;
    let raw = entry.get_password().ok()?;
    // New encoding: JSON-serialized Credential. Falls back to the legacy
    // `<user>\0<token>` two-field format if a credential was written before
    // the display_name / email fields existed.
    if let Ok(cred) = serde_json::from_str::<Credential>(&raw) {
        return Some(cred);
    }
    let mut parts = raw.splitn(2, '\0');
    let user = parts.next()?.to_string();
    let token = parts.next()?.to_string();
    Some(Credential {
        user,
        token,
        display_name: String::new(),
        email: String::new(),
    })
}

fn save_to_keychain(server_url: &str, cred: &Credential) -> bool {
    let entry = match keyring::Entry::new(KEYRING_SERVICE, server_url) {
        Ok(e) => e,
        Err(_) => return false,
    };
    let blob = match serde_json::to_string(cred) {
        Ok(s) => s,
        Err(_) => return false,
    };
    entry.set_password(&blob).is_ok()
}

fn delete_from_keychain(server_url: &str) -> bool {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, server_url) {
        return entry.delete_password().is_ok();
    }
    false
}

// ── File backend ─────────────────────────────────────────────────────────────

#[derive(Default, Serialize, Deserialize)]
struct CredentialFile {
    #[serde(flatten)]
    entries: BTreeMap<String, Credential>,
}

fn file_path() -> Result<PathBuf> {
    let home = home_dir().context("could not resolve home directory")?;
    Ok(home.join(".forge").join("credentials"))
}

fn home_dir() -> Option<PathBuf> {
    // No `home` crate dep — std env vars are enough on every platform we ship.
    if let Some(home) = std::env::var_os("HOME") {
        return Some(PathBuf::from(home));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return Some(PathBuf::from(profile));
    }
    None
}

fn load_from_file(server_url: &str) -> Result<Option<Credential>> {
    let path = match file_path() {
        Ok(p) => p,
        Err(_) => return Ok(None),
    };
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let parsed: CredentialFile =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(parsed.entries.get(server_url).cloned())
}

fn save_to_file(server_url: &str, cred: &Credential) -> Result<()> {
    let path = file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Read-modify-write so we don't drop other servers' credentials.
    let mut data: CredentialFile = if path.exists() {
        let raw = std::fs::read_to_string(&path)?;
        if raw.trim().is_empty() {
            CredentialFile::default()
        } else {
            serde_json::from_str(&raw).unwrap_or_default()
        }
    } else {
        CredentialFile::default()
    };
    data.entries.insert(server_url.to_string(), cred.clone());
    let json = serde_json::to_string_pretty(&data)?;
    std::fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;

    // 0600 on Unix. On Windows the keyring is the right backend; the file is
    // a fallback that we trust the user's profile ACLs for.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn delete_from_file(server_url: &str) -> Result<()> {
    let path = match file_path() {
        Ok(p) => p,
        Err(_) => return Ok(()),
    };
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(());
    }
    let mut data: CredentialFile = serde_json::from_str(&raw).unwrap_or_default();
    if data.entries.remove(server_url).is_some() {
        let json = serde_json::to_string_pretty(&data)?;
        std::fs::write(&path, json)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_url() -> String {
        // Each test gets its own server URL so they don't fight over keychain
        // entries on shared developer machines.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        format!("https://forge-test-{nanos}.invalid")
    }

    #[test]
    fn save_then_load_via_some_backend_round_trips() {
        let url = unique_url();
        let cred = Credential {
            user: "alice".into(),
            token: "fpat_dummy".into(),
            display_name: "Alice".into(),
            email: "alice@example.com".into(),
        };
        let backend = save(&url, &cred).unwrap();
        let _ = backend; // we don't care which one — both are valid
        let loaded = load(&url).unwrap().unwrap();
        assert_eq!(loaded.user, "alice");
        assert_eq!(loaded.token, "fpat_dummy");
        delete(&url).unwrap();
        assert!(load(&url).unwrap().is_none());
    }

    // Env var logic is tested via the pure `env_credential` helper instead
    // of mutating global env state, which would race with the threaded test
    // runner.

    #[test]
    fn env_credential_no_token_returns_none() {
        assert!(env_credential("https://x", None, None, None).is_none());
        assert!(env_credential("https://x", Some(String::new()), None, None).is_none());
    }

    #[test]
    fn env_credential_with_token_no_filter_applies_everywhere() {
        let cred = env_credential(
            "https://forge.acme.com",
            Some("fpat_x".into()),
            Some("ci".into()),
            None,
        )
        .unwrap();
        assert_eq!(cred.token, "fpat_x");
        assert_eq!(cred.user, "ci");
    }

    #[test]
    fn env_credential_with_matching_filter_applies() {
        let cred = env_credential(
            "https://forge.acme.com",
            Some("fpat_x".into()),
            None,
            Some("https://forge.acme.com".into()),
        )
        .unwrap();
        assert_eq!(cred.token, "fpat_x");
        assert_eq!(cred.user, "");
    }

    #[test]
    fn env_credential_with_mismatched_filter_is_ignored() {
        assert!(env_credential(
            "https://forge.acme.com",
            Some("fpat_x".into()),
            None,
            Some("https://other.invalid".into()),
        )
        .is_none());
    }
}
