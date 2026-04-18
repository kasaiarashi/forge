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

// ── Migration subcommand ─────────────────────────────────────────────────────

/// `forge-server migrate`: idempotent schema migration runner.
///
/// Reads `[database] backend = ...` from the config and applies any
/// pending migrations for the selected backend. A DB already at head
/// logs its current revision and exits 0.
///
/// For Postgres, `[database] url` must be set. Build the binary with
/// `--features postgres` to include the backend.
pub fn migrate(config: &ServerConfig) -> Result<()> {
    use crate::storage::backend::MetadataBackend;

    let backend_name = config.database.backend.as_str();
    match backend_name {
        "sqlite" => {
            let db_path = config.resolved_db_path();
            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            println!("SQLite backend: {}", db_path.display());
            // MetadataDb::open already applies pending migrations and
            // records the baseline. Calling apply_pending_migrations()
            // again just logs "no pending migrations" — still useful
            // because it prints the current version.
            let db = MetadataDb::open(&db_path)
                .with_context(|| format!("open metadata db at {}", db_path.display()))?;
            let before = db.current_schema_version()?;
            let applied = db.apply_pending_migrations()?;
            println!("schema_version before: {before}");
            println!("migrations applied:    {applied}");
            println!("schema_version after:  {}", db.current_schema_version()?);
        }
        "postgres" => {
            #[cfg(feature = "postgres")]
            {
                use crate::storage::postgres::{PgMetadataBackend, PgPoolConfig};
                if config.database.url.is_empty() {
                    bail!("postgres backend selected but [database] url is empty");
                }
                println!(
                    "Postgres backend: {}",
                    mask_url_password(&config.database.url)
                );
                let pg_cfg = PgPoolConfig {
                    url: config.database.url.clone(),
                    max_size: config.database.max_connections,
                    ..Default::default()
                };
                let backend = PgMetadataBackend::open(pg_cfg)
                    .context("open postgres backend")?;
                let before = backend.current_schema_version()?;
                // open() already applied pending migrations; re-running
                // is a no-op and prints the idempotent status line.
                let applied = backend.apply_pending_migrations()?;
                println!("schema_version before: {before}");
                println!("migrations applied:    {applied}");
                println!(
                    "schema_version after:  {}",
                    backend.current_schema_version()?
                );
            }
            #[cfg(not(feature = "postgres"))]
            {
                bail!(
                    "postgres backend requested but this binary was built without \
                     the `postgres` feature. Rebuild with `--features postgres`."
                );
            }
        }
        other => bail!("unknown [database] backend '{other}' (expected 'sqlite' or 'postgres')"),
    }
    Ok(())
}

// ── Repack subcommand ────────────────────────────────────────────────────────

/// `forge-server repack`: offline pack-builder. Consolidates small
/// loose objects into `<repo>/objects/packs/<uuid>.{pack,idx}` and
/// deletes the loose copies after the pack is durable on disk.
///
/// See [`crate::services::repack`] for the per-repo semantics. This
/// wrapper just resolves config, lists repos, and prints a per-repo
/// report table.
pub fn repack(
    config: &ServerConfig,
    dry_run: bool,
    max_loose_bytes: u64,
    repo: Option<&str>,
) -> Result<()> {
    use crate::services::repack;
    use crate::storage::fs::FsStorage;

    if config.database.backend.as_str() != "sqlite" {
        bail!(
            "forge-server repack currently supports only the sqlite backend \
             ([database] backend = \"sqlite\"). Postgres support lands alongside \
             the full-server trait migration."
        );
    }

    let base = config.storage.base_path.clone();
    std::fs::create_dir_all(base.join("repos")).ok();
    let db = open_db(config)?;
    let repo_overrides: std::collections::HashMap<String, std::path::PathBuf> = config
        .repos
        .iter()
        .filter_map(|(name, rc)| rc.path.as_ref().map(|p| (name.clone(), p.clone())))
        .collect();
    let fs = FsStorage::new(base.join("repos"), repo_overrides);

    let repos: Vec<String> = if let Some(name) = repo {
        vec![name.to_string()]
    } else {
        db.list_repos()?.into_iter().map(|r| r.name).collect()
    };

    let reports = repack::run(&fs, &repos, max_loose_bytes, dry_run)?;

    println!(
        "{:<32} {:>8} {:>8} {:>8} {:>10} {:>10} {:>14} {:>14} {:>7}",
        "REPO",
        "SCANNED",
        "PACKED",
        "LARGE",
        "DUP",
        "DELETED",
        "LOOSE_BYTES",
        "PACK_BYTES",
        "ERRORS"
    );
    println!("{}", "-".repeat(118));
    let mut total_packed = 0u64;
    let mut total_loose = 0u64;
    let mut total_pack = 0u64;
    let mut total_errors = 0u64;
    for r in &reports {
        total_packed += r.packed;
        total_loose += r.bytes_loose_before;
        total_pack += r.bytes_pack;
        total_errors += r.errors;
        println!(
            "{:<32} {:>8} {:>8} {:>8} {:>10} {:>10} {:>14} {:>14} {:>7}",
            truncate(&r.repo, 32),
            r.scanned,
            r.packed,
            r.skipped_large,
            r.already_packed,
            r.loose_deleted,
            r.bytes_loose_before,
            r.bytes_pack,
            r.errors,
        );
    }
    println!();
    println!(
        "Total: packed={total_packed}, loose_bytes_before={total_loose}, \
         pack_bytes={total_pack}, errors={total_errors}"
    );
    if dry_run {
        println!("(dry run — no packs written, no loose copies removed)");
    }
    Ok(())
}

// ── GC subcommand ────────────────────────────────────────────────────────────

/// `forge-server gc`: run a mark-and-sweep pass over every repo (or a
/// single repo via `--repo`). Intended for operators who want an
/// explicit reclaim window in addition to the scheduled sweep.
///
/// Refuses `postgres` backend for now — GC reads the metadata via
/// `MetadataDb` directly since the trait-covered surface is enough for
/// the push path but the CLI path also touches concrete helpers.
pub fn gc(
    config: &ServerConfig,
    dry_run: bool,
    grace_hours: i64,
    repo: Option<&str>,
) -> Result<()> {
    use crate::services::gc;
    use crate::storage::fs::FsStorage;

    if config.database.backend.as_str() != "sqlite" {
        bail!(
            "forge-server gc currently supports only the sqlite backend \
             ([database] backend = \"sqlite\"). Postgres GC lands in a \
             later phase once the server fully runs on the trait."
        );
    }

    let base = config.storage.base_path.clone();
    std::fs::create_dir_all(base.join("repos")).ok();
    let db = open_db(config)?;
    let repo_overrides: std::collections::HashMap<String, std::path::PathBuf> = config
        .repos
        .iter()
        .filter_map(|(name, rc)| rc.path.as_ref().map(|p| (name.clone(), p.clone())))
        .collect();
    let fs = FsStorage::new(base.join("repos"), repo_overrides);

    let grace_secs = grace_hours * 3600;

    let reports = if let Some(name) = repo {
        vec![gc::run_one(&db, &fs, name, grace_secs, dry_run)?]
    } else {
        gc::run(&db, &fs, grace_secs, dry_run)?
    };

    let mut total_swept = 0u64;
    let mut total_bytes = 0u64;
    let mut total_errors = 0u64;
    println!(
        "{:<32} {:>9} {:>9} {:>9} {:>9} {:>14} {:>7}",
        "REPO", "SCANNED", "MARKED", "SWEPT", "YOUNG", "BYTES", "ERRORS"
    );
    println!("{}", "-".repeat(96));
    for r in &reports {
        total_swept += r.swept;
        total_bytes += r.bytes_freed;
        total_errors += r.errors;
        println!(
            "{:<32} {:>9} {:>9} {:>9} {:>9} {:>14} {:>7}",
            truncate(&r.repo, 32),
            r.scanned,
            r.marked,
            r.swept,
            r.skipped_young,
            r.bytes_freed,
            r.errors,
        );
    }
    println!();
    println!(
        "Total: swept={total_swept}, bytes_freed={total_bytes}, errors={total_errors}"
    );
    if dry_run {
        println!("(dry run — nothing was deleted)");
    }
    if total_errors > 0 {
        println!(
            "Non-fatal errors occurred during GC; inspect the server log \
             for detail. Rerun after resolving before relying on disk \
             accounting."
        );
    }
    Ok(())
}

// ── Backup subcommand (Phase 7) ──────────────────────────────────────────────

/// Snapshot JSON written alongside the DB copy. Captures the minimum
/// context needed to sanity-check a restore: schema version, repo +
/// ref heads at the instant of the snapshot, and a human timestamp.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BackupManifest {
    pub created_at: String,
    pub server_version: String,
    pub schema_version: i64,
    pub backend: String,
    pub repos: Vec<BackupRepoEntry>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BackupRepoEntry {
    pub name: String,
    pub visibility: String,
    pub refs: Vec<BackupRefEntry>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BackupRefEntry {
    pub name: String,
    pub hash: String,
}

/// `forge-server backup create <dest>`.
///
/// Produces a **consistent** point-in-time DB snapshot via SQLite's
/// `VACUUM INTO` (safe to run against a live server) and writes a
/// sibling `manifest.json` listing repo + ref state. Object blobs are
/// content-addressed and immutable — operators back those up out-of-band
/// (rsync the FS tree, or `aws s3 sync` the bucket prefix); the CLI
/// would rather print the right command than lock users into a slow
/// in-binary copy. See docs/backup.md for the operator runbook.
pub fn backup_create(config: &ServerConfig, dest: &std::path::Path) -> Result<()> {
    let backend_name = config.database.backend.as_str();
    match backend_name {
        "sqlite" => {}
        "postgres" => bail!(
            "backup create is SQLite-only. For Postgres use pg_dump / pg_basebackup \
             against [database] url, and rsync the object store prefix alongside it."
        ),
        other => bail!("unknown [database] backend '{other}'"),
    }

    std::fs::create_dir_all(dest)
        .with_context(|| format!("create backup dir {}", dest.display()))?;

    let db_path = config.resolved_db_path();
    if !db_path.exists() {
        bail!("metadata db {} does not exist", db_path.display());
    }
    let db = MetadataDb::open(&db_path)
        .with_context(|| format!("open metadata db at {}", db_path.display()))?;

    let out_db = dest.join("forge.db");
    if out_db.exists() {
        bail!(
            "{} already exists; refusing to overwrite. Point --dest at a fresh dir.",
            out_db.display()
        );
    }

    // VACUUM INTO produces a single-file snapshot of the live DB without
    // holding a write lock on the source for long. Safe to run against
    // a running server.
    let conn = crate::storage::db::MetadataDb::conn(&db).context("borrow pooled conn")?;
    // sqlite3 VACUUM INTO syntax accepts a path literal. We escape the
    // single quote by doubling it — SQLite's rule.
    let escaped = out_db.to_string_lossy().replace('\'', "''");
    conn.execute_batch(&format!("VACUUM INTO '{escaped}';"))
        .with_context(|| format!("VACUUM INTO {}", out_db.display()))?;
    drop(conn);

    // Emit the manifest from the live DB (not the snapshot — the snapshot
    // is identical for our purposes, and using the live handle keeps the
    // code path simple).
    let mut repos: Vec<BackupRepoEntry> = Vec::new();
    for r in db.list_repos()? {
        let refs = db
            .get_all_refs(&r.name)
            .unwrap_or_default()
            .into_iter()
            .map(|(name, hash)| BackupRefEntry {
                name,
                hash: hex::encode(hash),
            })
            .collect();
        repos.push(BackupRepoEntry {
            name: r.name,
            visibility: r.visibility,
            refs,
        });
    }
    let manifest = BackupManifest {
        created_at: chrono::Utc::now().to_rfc3339(),
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version: db.current_schema_version()?,
        backend: backend_name.to_string(),
        repos,
    };
    let manifest_path = dest.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest)?,
    )
    .with_context(|| format!("write {}", manifest_path.display()))?;

    let size = std::fs::metadata(&out_db).map(|m| m.len()).unwrap_or(0);
    println!("wrote {} ({} bytes)", out_db.display(), size);
    println!("wrote {}", manifest_path.display());
    println!(
        "\nobject store is NOT included in this snapshot — back it up separately:"
    );
    match config.objects.backend.as_str() {
        "s3" => {
            println!("  aws s3 sync s3://{}/ s3://<backup-bucket>/", config.objects.s3.bucket);
        }
        _ => {
            let repos_root = config.storage.base_path.join("repos");
            println!("  rsync -a {} <backup-target>/", repos_root.display());
        }
    }
    Ok(())
}

/// `forge-server backup verify <path>`.
///
/// Opens the backup DB in read-only mode, runs `PRAGMA integrity_check`,
/// confirms the manifest matches, and reports the schema version. Safe
/// to run on a live backup dir.
pub fn backup_verify(path: &std::path::Path) -> Result<()> {
    let db_path = path.join("forge.db");
    let manifest_path = path.join("manifest.json");
    if !db_path.exists() {
        bail!("{} does not exist", db_path.display());
    }
    if !manifest_path.exists() {
        bail!("{} does not exist", manifest_path.display());
    }

    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open backup db {}", db_path.display()))?;

    let integrity: String = conn
        .prepare("PRAGMA integrity_check;")?
        .query_row([], |r| r.get(0))
        .unwrap_or_else(|_| "unknown".into());
    if integrity != "ok" {
        bail!("integrity_check failed: {integrity}");
    }

    let schema_version: i64 = conn
        .prepare("SELECT COALESCE(MAX(version), 0) FROM schema_version")?
        .query_row([], |r| r.get(0))?;
    let repo_count: i64 = conn
        .prepare("SELECT COUNT(*) FROM repos")?
        .query_row([], |r| r.get(0))?;
    let ref_count: i64 = conn
        .prepare("SELECT COUNT(*) FROM refs")?
        .query_row([], |r| r.get(0))?;

    let manifest: BackupManifest = serde_json::from_slice(
        &std::fs::read(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))?,
    )
    .with_context(|| format!("parse {}", manifest_path.display()))?;

    // Cross-check: manifest's repo count and schema_version must match
    // the DB file. A mismatch means the archive was tampered with or
    // copied inconsistently.
    let manifest_refs: i64 = manifest.repos.iter().map(|r| r.refs.len() as i64).sum();
    if manifest.schema_version != schema_version {
        bail!(
            "schema_version mismatch: manifest = {}, db = {schema_version}",
            manifest.schema_version,
        );
    }
    if manifest.repos.len() as i64 != repo_count {
        bail!(
            "repo count mismatch: manifest = {}, db = {repo_count}",
            manifest.repos.len(),
        );
    }
    if manifest_refs != ref_count {
        bail!("ref count mismatch: manifest = {manifest_refs}, db = {ref_count}");
    }

    println!("integrity_check:  ok");
    println!("schema_version:   {schema_version}");
    println!("repos:            {repo_count}");
    println!("refs:             {ref_count}");
    println!("created_at:       {}", manifest.created_at);
    println!("server_version:   {}", manifest.server_version);
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n.saturating_sub(1)])
    }
}

#[cfg(feature = "postgres")]
fn mask_url_password(url: &str) -> String {
    // Avoid leaking a libpq password to operator logs. Splits on `:` +
    // `@` and replaces the password chunk with `***`. Best-effort —
    // unparseable URLs pass through unchanged so an operator sees the
    // literal value and can debug their config.
    if let Some((prefix, rest)) = url.split_once("://") {
        if let Some((creds, host)) = rest.split_once('@') {
            if let Some((user, _pw)) = creds.split_once(':') {
                return format!("{prefix}://{user}:***@{host}");
            }
        }
    }
    url.to_string()
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

