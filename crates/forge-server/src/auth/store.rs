// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! User, session, PAT, and per-repo ACL persistence.
//!
//! [`UserStore`] is the abstraction the rest of the server talks to. The v1
//! implementation [`SqliteUserStore`] backs it with the existing
//! [`MetadataDb`](crate::storage::db::MetadataDb) connection so all auth state
//! lives in the same SQLite file as repos/refs/locks (one DB to back up).
//!
//! Future identity backends (OIDC, SSH-key, LDAP) plug in by implementing the
//! same trait. Callers throughout the server hold an `Arc<dyn UserStore>` so
//! the choice is runtime, not compile-time.
//!
//! ## Synchronous on purpose
//!
//! The trait is synchronous. Rusqlite is synchronous, the per-request DB cost
//! is microseconds, and gRPC handlers in tonic call interceptors on a worker
//! thread already — there's nothing to gain by `async`-wrapping a `Mutex`.
//! When the interceptor adds an in-memory token cache (phase 3), the cached
//! path won't even hit this layer.

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;

use super::password;
use super::tokens::{self, PatPlaintext, Scope};
use crate::storage::db::MetadataDb;

// ── Domain types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub is_server_admin: bool,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewUser {
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub password: String,
    pub is_server_admin: bool,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: i64,
    pub user_id: i64,
    pub created_at: i64,
    pub last_used_at: i64,
    pub expires_at: i64,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
}

/// The plaintext + persisted form of a freshly-minted session token. The
/// plaintext is set as an HttpOnly cookie by the web UI; the CLI does the
/// same when storing it locally.
#[derive(Debug, Clone)]
pub struct SessionToken {
    pub session: Session,
    /// Plaintext to hand to the client; never persisted.
    pub plaintext: String,
}

#[derive(Debug, Clone)]
pub struct PersonalAccessToken {
    pub id: i64,
    pub name: String,
    pub user_id: i64,
    pub scopes: Vec<Scope>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoRole {
    Read,
    Write,
    Admin,
}

impl RepoRole {
    pub fn as_str(self) -> &'static str {
        match self {
            RepoRole::Read => "read",
            RepoRole::Write => "write",
            RepoRole::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "admin" => Ok(Self::Admin),
            other => Err(anyhow!("unknown repo role '{other}'")),
        }
    }

    /// Role hierarchy: admin > write > read. Returns true if `self` is at
    /// least as permissive as `needed`.
    pub fn satisfies(self, needed: RepoRole) -> bool {
        let level = |r: RepoRole| match r {
            RepoRole::Read => 1,
            RepoRole::Write => 2,
            RepoRole::Admin => 3,
        };
        level(self) >= level(needed)
    }
}

// ── The trait ────────────────────────────────────────────────────────────────

pub trait UserStore: Send + Sync {
    // Users
    fn create_user(&self, input: NewUser) -> Result<User>;
    fn find_user_by_username(&self, username: &str) -> Result<Option<User>>;
    fn find_user_by_id(&self, id: i64) -> Result<Option<User>>;
    fn list_users(&self) -> Result<Vec<User>>;
    fn delete_user(&self, id: i64) -> Result<bool>;
    fn count_users(&self) -> Result<i64>;
    fn set_password(&self, user_id: i64, new_password: &str) -> Result<()>;

    /// Verify a username/password against the local-password backend. Returns
    /// the user on success, `None` on bad credentials. Updates `last_login_at`
    /// on success.
    fn verify_password(&self, username: &str, password: &str) -> Result<Option<User>>;

    // Sessions
    fn create_session(
        &self,
        user_id: i64,
        ttl_seconds: i64,
        user_agent: Option<&str>,
        ip: Option<&str>,
    ) -> Result<SessionToken>;
    fn find_session_by_plaintext(&self, plaintext: &str) -> Result<Option<(Session, User)>>;
    fn list_sessions_for_user(&self, user_id: i64) -> Result<Vec<Session>>;
    fn revoke_session(&self, session_id: i64) -> Result<bool>;
    fn touch_session(&self, session_id: i64) -> Result<()>;

    // PATs
    fn create_pat(
        &self,
        user_id: i64,
        name: &str,
        scopes: &[Scope],
        expires_at: Option<i64>,
    ) -> Result<(PersonalAccessToken, PatPlaintext)>;
    fn find_pat_by_plaintext(&self, plaintext: &str)
        -> Result<Option<(PersonalAccessToken, User)>>;
    fn list_pats_for_user(&self, user_id: i64) -> Result<Vec<PersonalAccessToken>>;
    fn revoke_pat(&self, pat_id: i64) -> Result<bool>;
    fn touch_pat(&self, pat_id: i64) -> Result<()>;

    // ACLs
    fn get_repo_role(&self, repo: &str, user_id: i64) -> Result<Option<RepoRole>>;
    /// Grant or update a repo role. `granted_by` is the user id that's
    /// recording the grant for audit purposes; pass `None` for grants that
    /// originate from the operator-side `forge-server repo grant` CLI (where
    /// there is no authenticated caller).
    fn set_repo_role(
        &self,
        repo: &str,
        user_id: i64,
        role: RepoRole,
        granted_by: Option<i64>,
    ) -> Result<()>;
    fn revoke_repo_role(&self, repo: &str, user_id: i64) -> Result<bool>;
    fn list_repo_members(&self, repo: &str) -> Result<Vec<(User, RepoRole)>>;
}

// ── SqliteUserStore ──────────────────────────────────────────────────────────

/// SQLite-backed implementation of [`UserStore`]. Shares the same connection
/// as the rest of the server's metadata DB.
pub struct SqliteUserStore {
    db: Arc<MetadataDb>,
}

impl SqliteUserStore {
    pub fn new(db: Arc<MetadataDb>) -> Self {
        Self { db }
    }

    /// Test-only accessor for the underlying [`MetadataDb`]. Used by the
    /// integration tests in `auth::tests` to set up edge cases (forced
    /// expiry, FK pragmas) without polluting the public surface.
    #[cfg(test)]
    pub(crate) fn db(&self) -> &MetadataDb {
        &self.db
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn map_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        email: row.get(2)?,
        display_name: row.get(3)?,
        is_server_admin: row.get::<_, i64>(4)? != 0,
        created_at: row.get(5)?,
        last_login_at: row.get(6)?,
    })
}

const USER_COLUMNS: &str =
    "id, username, email, display_name, is_server_admin, created_at, last_login_at";

fn map_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        user_id: row.get(1)?,
        created_at: row.get(2)?,
        last_used_at: row.get(3)?,
        expires_at: row.get(4)?,
        user_agent: row.get(5)?,
        ip: row.get(6)?,
    })
}

const SESSION_COLUMNS: &str = "id, user_id, created_at, last_used_at, expires_at, user_agent, ip";

fn map_pat(row: &rusqlite::Row<'_>) -> Result<PersonalAccessToken> {
    let scopes_str: String = row.get(4).map_err(|e| anyhow!("get scopes: {e}"))?;
    Ok(PersonalAccessToken {
        id: row.get(0).map_err(|e| anyhow!("get id: {e}"))?,
        name: row.get(1).map_err(|e| anyhow!("get name: {e}"))?,
        user_id: row.get(2).map_err(|e| anyhow!("get user_id: {e}"))?,
        scopes: tokens::parse_scopes(&scopes_str).context("decode pat scopes")?,
        created_at: row.get(3).map_err(|e| anyhow!("get created_at: {e}"))?,
        last_used_at: row.get(5).map_err(|e| anyhow!("get last_used_at: {e}"))?,
        expires_at: row.get(6).map_err(|e| anyhow!("get expires_at: {e}"))?,
    })
}

const PAT_COLUMNS: &str = "id, name, user_id, created_at, scopes, last_used_at, expires_at";

impl UserStore for SqliteUserStore {
    // ── Users ───────────────────────────────────────────────────────────────

    fn create_user(&self, input: NewUser) -> Result<User> {
        if input.username.is_empty() {
            bail!("username is required");
        }
        if input.email.is_empty() {
            bail!("email is required");
        }
        if input.password.is_empty() {
            bail!("password is required");
        }
        let display_name = if input.display_name.is_empty() {
            input.username.clone()
        } else {
            input.display_name.clone()
        };
        let pw_hash = password::hash(&input.password)?;
        let conn = self.db.conn()?;
        let now = now();
        conn.execute(
            "INSERT INTO users (username, email, display_name, password_hash, is_server_admin, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                input.username,
                input.email,
                display_name,
                pw_hash,
                if input.is_server_admin { 1 } else { 0 },
                now
            ],
        )
        .with_context(|| format!("insert user {}", input.username))?;
        let id = conn.last_insert_rowid();
        Ok(User {
            id,
            username: input.username,
            email: input.email,
            display_name,
            is_server_admin: input.is_server_admin,
            created_at: now,
            last_login_at: None,
        })
    }

    fn find_user_by_username(&self, username: &str) -> Result<Option<User>> {
        let conn = self.db.conn()?;
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE username = ?1");
        let mut stmt = conn.prepare(&sql)?;
        let result = stmt.query_row(params![username], map_user).optional()?;
        Ok(result)
    }

    fn find_user_by_id(&self, id: i64) -> Result<Option<User>> {
        let conn = self.db.conn()?;
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?1");
        let mut stmt = conn.prepare(&sql)?;
        let result = stmt.query_row(params![id], map_user).optional()?;
        Ok(result)
    }

    fn list_users(&self) -> Result<Vec<User>> {
        let conn = self.db.conn()?;
        let sql = format!("SELECT {USER_COLUMNS} FROM users ORDER BY username");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], map_user)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn delete_user(&self, id: i64) -> Result<bool> {
        let conn = self.db.conn()?;
        // CASCADE handles sessions, pats, repo_acls.
        let n = conn.execute("DELETE FROM users WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    fn count_users(&self) -> Result<i64> {
        let conn = self.db.conn()?;
        let n: i64 = conn
            .prepare("SELECT COUNT(*) FROM users")?
            .query_row([], |row| row.get(0))?;
        Ok(n)
    }

    fn set_password(&self, user_id: i64, new_password: &str) -> Result<()> {
        if new_password.is_empty() {
            bail!("password cannot be empty");
        }
        let hash = password::hash(new_password)?;
        let conn = self.db.conn()?;
        let n = conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            params![hash, user_id],
        )?;
        if n == 0 {
            bail!("user {user_id} not found");
        }
        Ok(())
    }

    fn verify_password(&self, username: &str, password: &str) -> Result<Option<User>> {
        let conn = self.db.conn()?;
        let sql = format!("SELECT {USER_COLUMNS}, password_hash FROM users WHERE username = ?1");
        let row = conn
            .prepare(&sql)?
            .query_row(params![username], |row| {
                let user = map_user(row)?;
                let hash: Option<String> = row.get(7)?;
                Ok((user, hash))
            })
            .optional()?;
        let (user, hash) = match row {
            Some(r) => r,
            None => return Ok(None),
        };
        let hash = match hash {
            Some(h) => h,
            // User exists but has no password (e.g. future OIDC-only user) —
            // verify_password is the wrong path for them; treat as no match.
            None => return Ok(None),
        };
        if !password::verify(password, &hash)? {
            return Ok(None);
        }
        // Touch last_login_at — best effort, never fails the login.
        let _ = conn.execute(
            "UPDATE users SET last_login_at = ?1 WHERE id = ?2",
            params![now(), user.id],
        );
        Ok(Some(user))
    }

    // ── Sessions ────────────────────────────────────────────────────────────

    fn create_session(
        &self,
        user_id: i64,
        ttl_seconds: i64,
        user_agent: Option<&str>,
        ip: Option<&str>,
    ) -> Result<SessionToken> {
        if ttl_seconds <= 0 {
            bail!("session ttl must be positive");
        }
        let token = tokens::generate_session()?;
        let conn = self.db.conn()?;
        let now = now();
        let expires_at = now + ttl_seconds;
        conn.execute(
            "INSERT INTO sessions (token_hash, token_prefix, user_id, created_at, last_used_at, expires_at, user_agent, ip)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                token.hash,
                token.prefix,
                user_id,
                now,
                now,
                expires_at,
                user_agent,
                ip
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(SessionToken {
            session: Session {
                id,
                user_id,
                created_at: now,
                last_used_at: now,
                expires_at,
                user_agent: user_agent.map(str::to_string),
                ip: ip.map(str::to_string),
            },
            plaintext: token.plaintext,
        })
    }

    fn find_session_by_plaintext(&self, plaintext: &str) -> Result<Option<(Session, User)>> {
        let prefix = tokens::prefix_of(plaintext);
        let conn = self.db.conn()?;
        let sql = format!(
            "SELECT s.{}, s.token_hash, u.{}
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_prefix = ?1",
            SESSION_COLUMNS.split(", ").collect::<Vec<_>>().join(", s."),
            USER_COLUMNS.split(", ").collect::<Vec<_>>().join(", u.")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![prefix], |row| {
            let session = map_session(row)?;
            let token_hash: String = row.get(7)?;
            // user columns start after the 7 session columns + 1 token_hash
            let user = User {
                id: row.get(8)?,
                username: row.get(9)?,
                email: row.get(10)?,
                display_name: row.get(11)?,
                is_server_admin: row.get::<_, i64>(12)? != 0,
                created_at: row.get(13)?,
                last_login_at: row.get(14)?,
            };
            Ok((session, token_hash, user))
        })?;
        let now_ts = now();
        for row in rows {
            let (session, hash, user) = row?;
            if session.expires_at <= now_ts {
                continue; // expired — skip
            }
            if password::verify(plaintext, &hash)? {
                return Ok(Some((session, user)));
            }
        }
        Ok(None)
    }

    fn list_sessions_for_user(&self, user_id: i64) -> Result<Vec<Session>> {
        let conn = self.db.conn()?;
        let sql = format!(
            "SELECT {SESSION_COLUMNS} FROM sessions WHERE user_id = ?1 ORDER BY last_used_at DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![user_id], map_session)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn revoke_session(&self, session_id: i64) -> Result<bool> {
        let conn = self.db.conn()?;
        let n = conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
        Ok(n > 0)
    }

    fn touch_session(&self, session_id: i64) -> Result<()> {
        let conn = self.db.conn()?;
        conn.execute(
            "UPDATE sessions SET last_used_at = ?1 WHERE id = ?2",
            params![now(), session_id],
        )?;
        Ok(())
    }

    // ── PATs ────────────────────────────────────────────────────────────────

    fn create_pat(
        &self,
        user_id: i64,
        name: &str,
        scopes: &[Scope],
        expires_at: Option<i64>,
    ) -> Result<(PersonalAccessToken, PatPlaintext)> {
        if name.is_empty() {
            bail!("token name is required");
        }
        tokens::validate_scopes(scopes)?;
        let token = tokens::generate_pat()?;
        let scopes_str = tokens::encode_scopes(scopes);
        let conn = self.db.conn()?;
        let now_ts = now();
        conn.execute(
            "INSERT INTO personal_access_tokens (name, token_hash, token_prefix, user_id, scopes, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![name, token.hash, token.prefix, user_id, scopes_str, now_ts, expires_at],
        )?;
        let id = conn.last_insert_rowid();
        let pat = PersonalAccessToken {
            id,
            name: name.to_string(),
            user_id,
            scopes: scopes.to_vec(),
            created_at: now_ts,
            last_used_at: None,
            expires_at,
        };
        Ok((pat, token))
    }

    fn find_pat_by_plaintext(
        &self,
        plaintext: &str,
    ) -> Result<Option<(PersonalAccessToken, User)>> {
        let prefix = tokens::prefix_of(plaintext);
        let conn = self.db.conn()?;
        let sql = format!(
            "SELECT p.{}, p.token_hash, u.{}
             FROM personal_access_tokens p JOIN users u ON u.id = p.user_id
             WHERE p.token_prefix = ?1",
            PAT_COLUMNS.split(", ").collect::<Vec<_>>().join(", p."),
            USER_COLUMNS.split(", ").collect::<Vec<_>>().join(", u.")
        );
        let mut stmt = conn.prepare(&sql)?;
        // Collect rows first because map_pat returns anyhow::Result, which
        // doesn't fit cleanly inside rusqlite::Row's lifetime.
        let mut candidates: Vec<(PersonalAccessToken, String, User)> = Vec::new();
        let mut rows = stmt.query(params![prefix])?;
        while let Some(row) = rows.next()? {
            let pat = map_pat(row)?;
            let token_hash: String = row.get(7)?;
            let user = User {
                id: row.get(8)?,
                username: row.get(9)?,
                email: row.get(10)?,
                display_name: row.get(11)?,
                is_server_admin: row.get::<_, i64>(12)? != 0,
                created_at: row.get(13)?,
                last_login_at: row.get(14)?,
            };
            candidates.push((pat, token_hash, user));
        }
        let now_ts = now();
        for (pat, hash, user) in candidates {
            if let Some(exp) = pat.expires_at {
                if exp <= now_ts {
                    continue;
                }
            }
            if password::verify(plaintext, &hash)? {
                return Ok(Some((pat, user)));
            }
        }
        Ok(None)
    }

    fn list_pats_for_user(&self, user_id: i64) -> Result<Vec<PersonalAccessToken>> {
        let conn = self.db.conn()?;
        let sql = format!(
            "SELECT {PAT_COLUMNS} FROM personal_access_tokens WHERE user_id = ?1 ORDER BY created_at DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(params![user_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(map_pat(row)?);
        }
        Ok(out)
    }

    fn revoke_pat(&self, pat_id: i64) -> Result<bool> {
        let conn = self.db.conn()?;
        let n = conn.execute(
            "DELETE FROM personal_access_tokens WHERE id = ?1",
            params![pat_id],
        )?;
        Ok(n > 0)
    }

    fn touch_pat(&self, pat_id: i64) -> Result<()> {
        let conn = self.db.conn()?;
        conn.execute(
            "UPDATE personal_access_tokens SET last_used_at = ?1 WHERE id = ?2",
            params![now(), pat_id],
        )?;
        Ok(())
    }

    // ── ACLs ────────────────────────────────────────────────────────────────

    fn get_repo_role(&self, repo: &str, user_id: i64) -> Result<Option<RepoRole>> {
        let conn = self.db.conn()?;
        let role: Option<String> = conn
            .prepare("SELECT role FROM repo_acls WHERE repo = ?1 AND user_id = ?2")?
            .query_row(params![repo, user_id], |row| row.get(0))
            .optional()?;
        match role {
            Some(s) => Ok(Some(RepoRole::parse(&s)?)),
            None => Ok(None),
        }
    }

    fn set_repo_role(
        &self,
        repo: &str,
        user_id: i64,
        role: RepoRole,
        granted_by: Option<i64>,
    ) -> Result<()> {
        let conn = self.db.conn()?;
        conn.execute(
            "INSERT INTO repo_acls (repo, user_id, role, granted_at, granted_by)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(repo, user_id) DO UPDATE SET role = excluded.role, granted_at = excluded.granted_at, granted_by = excluded.granted_by",
            params![repo, user_id, role.as_str(), now(), granted_by],
        )?;
        Ok(())
    }

    fn revoke_repo_role(&self, repo: &str, user_id: i64) -> Result<bool> {
        let conn = self.db.conn()?;
        let n = conn.execute(
            "DELETE FROM repo_acls WHERE repo = ?1 AND user_id = ?2",
            params![repo, user_id],
        )?;
        Ok(n > 0)
    }

    fn list_repo_members(&self, repo: &str) -> Result<Vec<(User, RepoRole)>> {
        let conn = self.db.conn()?;
        let sql = format!(
            "SELECT u.{USER_COLUMNS}, a.role
             FROM repo_acls a JOIN users u ON u.id = a.user_id
             WHERE a.repo = ?1
             ORDER BY u.username"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(params![repo])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let user = map_user(row)?;
            let role_str: String = row.get(7)?;
            out.push((user, RepoRole::parse(&role_str)?));
        }
        Ok(out)
    }
}
