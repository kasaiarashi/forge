// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! `forge-server user …` and `forge-server repo …` subcommands.
//!
//! These talk directly to [`SqliteUserStore`] — no gRPC round-trip — so the
//! commands work whether or not the server process is running. This is the
//! documented path for the first-admin bootstrap on a fresh install:
//!
//! ```text
//! forge-server init
//! forge-server user add --admin alice
//! forge-server serve
//! ```

use anyhow::{bail, Context, Result};
use std::sync::Arc;

use crate::auth::{NewUser, RepoRole, SqliteUserStore, UserStore};
use crate::config::ServerConfig;
use crate::storage::db::MetadataDb;

/// Open the metadata DB at the location the loaded config points at and wrap
/// it in a [`SqliteUserStore`]. Used by every admin subcommand.
fn open_store(config: &ServerConfig) -> Result<SqliteUserStore> {
    let db_path = config.resolved_db_path();
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let db = MetadataDb::open(&db_path)
        .with_context(|| format!("open metadata db at {}", db_path.display()))?;
    Ok(SqliteUserStore::new(Arc::new(db)))
}

// ── User subcommands ─────────────────────────────────────────────────────────

pub fn user_add(
    config: &ServerConfig,
    username: &str,
    email: Option<&str>,
    display_name: Option<&str>,
    is_admin: bool,
    password: Option<&str>,
) -> Result<()> {
    let store = open_store(config)?;

    if username.is_empty() {
        bail!("username is required");
    }

    let email = match email {
        Some(e) => e.to_string(),
        None => prompt_line(&format!("Email for {username}: "))?,
    };
    if email.is_empty() {
        bail!("email is required");
    }

    let display_name = display_name
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| username.to_string());

    let password = match password {
        Some(p) => p.to_string(),
        None => prompt_password_with_confirm()?,
    };

    let user = store.create_user(NewUser {
        username: username.to_string(),
        email,
        display_name,
        password,
        is_server_admin: is_admin,
    })?;

    let badge = if user.is_server_admin {
        " (server admin)"
    } else {
        ""
    };
    println!("Created user '{}'{badge}", user.username);
    println!("  id:    {}", user.id);
    println!("  email: {}", user.email);
    Ok(())
}

pub fn user_list(config: &ServerConfig) -> Result<()> {
    let store = open_store(config)?;
    let users = store.list_users()?;
    if users.is_empty() {
        println!("No users yet. Create one with: forge-server user add --admin <username>");
        return Ok(());
    }
    println!(
        "{:<20} {:<6} {:<30} {}",
        "USERNAME", "ADMIN", "EMAIL", "DISPLAY NAME"
    );
    for u in users {
        println!(
            "{:<20} {:<6} {:<30} {}",
            u.username,
            if u.is_server_admin { "yes" } else { "no" },
            u.email,
            u.display_name
        );
    }
    Ok(())
}

pub fn user_delete(config: &ServerConfig, username: &str) -> Result<()> {
    let store = open_store(config)?;
    let user = store
        .find_user_by_username(username)?
        .ok_or_else(|| anyhow::anyhow!("user '{username}' not found"))?;
    let removed = store.delete_user(user.id)?;
    if removed {
        println!("Deleted user '{username}' (id {})", user.id);
        println!("Cascaded: their sessions, PATs, and repo ACL grants were removed.");
    } else {
        bail!("delete failed for user '{username}'");
    }
    Ok(())
}

pub fn user_reset_password(
    config: &ServerConfig,
    username: &str,
    password: Option<&str>,
) -> Result<()> {
    let store = open_store(config)?;
    let user = store
        .find_user_by_username(username)?
        .ok_or_else(|| anyhow::anyhow!("user '{username}' not found"))?;
    let new_password = match password {
        Some(p) => p.to_string(),
        None => prompt_password_with_confirm()?,
    };
    store.set_password(user.id, &new_password)?;
    println!("Password updated for '{username}'");
    println!("All existing sessions for this user remain valid until they expire or are revoked.");
    Ok(())
}

// ── Repo subcommands ─────────────────────────────────────────────────────────

pub fn repo_grant(
    config: &ServerConfig,
    repo: &str,
    username: &str,
    role: &str,
) -> Result<()> {
    let store = open_store(config)?;
    let target = store
        .find_user_by_username(username)?
        .ok_or_else(|| anyhow::anyhow!("user '{username}' not found"))?;
    let parsed_role = RepoRole::parse(role)
        .with_context(|| format!("'{role}' is not a valid role (read | write | admin)"))?;
    // CLI grants are issued by the server operator — no authenticated caller,
    // so granted_by is NULL. gRPC-issued grants in phase 3 will pass
    // Some(caller.user_id) for audit attribution.
    store.set_repo_role(repo, target.id, parsed_role, None)?;
    println!(
        "Granted '{}' role on '{}' to '{}' (id {})",
        parsed_role.as_str(),
        repo,
        username,
        target.id
    );
    Ok(())
}

pub fn repo_revoke(config: &ServerConfig, repo: &str, username: &str) -> Result<()> {
    let store = open_store(config)?;
    let target = store
        .find_user_by_username(username)?
        .ok_or_else(|| anyhow::anyhow!("user '{username}' not found"))?;
    let removed = store.revoke_repo_role(repo, target.id)?;
    if removed {
        println!("Revoked '{username}' from '{repo}'");
    } else {
        println!("'{username}' had no role on '{repo}' — nothing to do");
    }
    Ok(())
}

pub fn repo_list_members(config: &ServerConfig, repo: &str) -> Result<()> {
    let store = open_store(config)?;
    let members = store.list_repo_members(repo)?;
    if members.is_empty() {
        println!("'{repo}' has no granted members yet.");
        println!("Server admins can still access it. Grant a user with:");
        println!("  forge-server repo grant {repo} <username> <read|write|admin>");
        return Ok(());
    }
    println!("Members of '{repo}':");
    println!("  {:<20} {}", "USERNAME", "ROLE");
    for (user, role) in members {
        println!("  {:<20} {}", user.username, role.as_str());
    }
    Ok(())
}

// ── Bootstrap helper used by `serve` ─────────────────────────────────────────

/// Returns true if the database has at least one user. The web `/setup`
/// wizard checks this in phase 5 to decide whether to render the setup form.
#[allow(dead_code)] // consumed by phase 5
pub fn is_initialized(config: &ServerConfig) -> Result<bool> {
    let store = open_store(config)?;
    Ok(store.count_users()? > 0)
}

// ── Prompt helpers ───────────────────────────────────────────────────────────

fn prompt_line(prompt: &str) -> Result<String> {
    use std::io::Write;
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

// ── Agent subcommands ────────────────────────────────────────────────────────

fn open_db(config: &ServerConfig) -> Result<Arc<MetadataDb>> {
    let db_path = config.resolved_db_path();
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let db = MetadataDb::open(&db_path)
        .with_context(|| format!("open metadata db at {}", db_path.display()))?;
    Ok(Arc::new(db))
}

pub fn agent_add(config: &ServerConfig, name: &str, labels: &[String]) -> Result<()> {
    use argon2::{password_hash::{PasswordHasher, SaltString}, Argon2};
    use rand::RngCore;

    if name.is_empty() {
        bail!("agent name is required");
    }
    let db = open_db(config)?;

    // Random 32-byte token, hex-encoded. Stored only as Argon2 hash on the
    // server; the plaintext is printed once and expected to land in the
    // agent's keyring via `forge-agent register`.
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    let token = hex::encode(raw);
    let salt = SaltString::generate(&mut rand::thread_rng());
    let hash = Argon2::default()
        .hash_password(token.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("argon2: {e}"))?
        .to_string();

    let labels_json = serde_json::to_string(labels).unwrap_or_else(|_| "[]".into());
    let agent_id = db.upsert_agent(name, &hash, &labels_json, "", "")?;

    println!("Agent '{name}' provisioned (id {agent_id}).");
    println!("\n*** AGENT TOKEN — COPY NOW, WILL NOT BE SHOWN AGAIN ***");
    println!("    {token}");
    println!("\nRegister the agent with:");
    println!("    forge-agent register --server <URL> --name {name} --token {token}");
    Ok(())
}

pub fn agent_list(config: &ServerConfig) -> Result<()> {
    let db = open_db(config)?;
    let rows = db.list_agents()?;
    if rows.is_empty() {
        println!("No agents registered.");
        return Ok(());
    }
    println!("{:<5} {:<24} {:<8} {:<32} {}", "ID", "NAME", "OS", "LABELS", "LAST SEEN");
    println!("{}", "-".repeat(96));
    for (id, name, labels_json, last_seen, _version, os) in &rows {
        let labels: Vec<String> =
            serde_json::from_str(labels_json).unwrap_or_default();
        let when = if *last_seen == 0 {
            "never".to_string()
        } else {
            chrono::DateTime::from_timestamp(*last_seen, 0)
                .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "?".into())
        };
        println!(
            "{:<5} {:<24} {:<8} {:<32} {}",
            id,
            name,
            if os.is_empty() { "-" } else { os.as_str() },
            labels.join(","),
            when
        );
    }
    Ok(())
}

pub fn agent_remove(config: &ServerConfig, name: &str) -> Result<()> {
    let db = open_db(config)?;
    let row = db.get_agent_by_name(name)?;
    let (id, _, _) = match row {
        Some(r) => r,
        None => bail!("agent '{name}' not found"),
    };
    let removed = db.delete_agent(id)?;
    if removed {
        println!("Removed agent '{name}' (id {id}). Its token is no longer valid.");
    } else {
        println!("Nothing to remove.");
    }
    Ok(())
}

fn prompt_password_with_confirm() -> Result<String> {
    let p1 = rpassword::prompt_password("Password: ")?;
    if p1.is_empty() {
        bail!("password cannot be empty");
    }
    let p2 = rpassword::prompt_password("Confirm:  ")?;
    if p1 != p2 {
        bail!("passwords did not match");
    }
    Ok(p1)
}

