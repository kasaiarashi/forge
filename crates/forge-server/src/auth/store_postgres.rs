// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under BSL 1.1.

//! Postgres-backed [`UserStore`] implementation.
//!
//! Mirrors [`super::store::SqliteUserStore`] method-for-method so the
//! gRPC handlers don't care which backend is selected at startup. The
//! parity test suite runs both impls through the same scenarios when
//! `DATABASE_URL` is set + the `postgres-tests` feature is on.
//!
//! SQL dialect notes:
//! - Placeholders are `$N`, not `?N`.
//! - `INSERT OR IGNORE` becomes `INSERT … ON CONFLICT DO NOTHING`.
//! - `last_insert_rowid()` is replaced by `RETURNING id`.
//! - Booleans stay INTEGER (0/1) so the trait return type is one
//!   `bool` regardless of backend; keeps callers dialect-free.

#[cfg(feature = "postgres")]
use anyhow::{anyhow, bail, Context, Result};
#[cfg(feature = "postgres")]
use std::sync::Arc;

#[cfg(feature = "postgres")]
use super::password;
#[cfg(feature = "postgres")]
use super::store::{
    NewUser, PersonalAccessToken, RepoRole, Session, SessionToken, User, UserStore,
};
#[cfg(feature = "postgres")]
use super::tokens::{self, PatPlaintext, Scope};
#[cfg(feature = "postgres")]
use crate::storage::postgres::{PgMetadataBackend, PgPool};

#[cfg(feature = "postgres")]
pub struct PgUserStore {
    pool: PgPool,
}

#[cfg(feature = "postgres")]
impl PgUserStore {
    /// Construct a `PgUserStore` sharing the pool that
    /// [`PgMetadataBackend`] already opened. We do not open a second
    /// pool against the same database — `r2d2_postgres` handles
    /// concurrency adequately, and a single shared pool means the
    /// operator only has to size one knob.
    pub fn new(pg: Arc<PgMetadataBackend>) -> Self {
        Self { pool: pg.pool() }
    }

    fn conn(
        &self,
    ) -> Result<r2d2::PooledConnection<r2d2_postgres::PostgresConnectionManager<postgres::NoTls>>>
    {
        self.pool.get().context("postgres pool get (auth)")
    }
}

#[cfg(feature = "postgres")]
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(feature = "postgres")]
fn map_user_row(row: &postgres::Row) -> User {
    User {
        id: row.get("id"),
        username: row.get("username"),
        email: row.get("email"),
        display_name: row.get("display_name"),
        is_server_admin: row.get::<_, i32>("is_server_admin") != 0,
        created_at: row.get("created_at"),
        last_login_at: row.get("last_login_at"),
    }
}

#[cfg(feature = "postgres")]
fn map_session_row(row: &postgres::Row) -> Session {
    Session {
        id: row.get("id"),
        user_id: row.get("user_id"),
        created_at: row.get("created_at"),
        last_used_at: row.get("last_used_at"),
        expires_at: row.get("expires_at"),
        user_agent: row.get("user_agent"),
        ip: row.get("ip"),
    }
}

#[cfg(feature = "postgres")]
fn map_pat_row(row: &postgres::Row) -> Result<PersonalAccessToken> {
    let scopes_str: String = row.get("scopes");
    Ok(PersonalAccessToken {
        id: row.get("id"),
        name: row.get("name"),
        user_id: row.get("user_id"),
        scopes: tokens::parse_scopes(&scopes_str).context("decode pat scopes")?,
        created_at: row.get("created_at"),
        last_used_at: row.get("last_used_at"),
        expires_at: row.get("expires_at"),
    })
}

#[cfg(feature = "postgres")]
const USER_COLUMNS: &str =
    "id, username, email, display_name, is_server_admin, created_at, last_login_at";

#[cfg(feature = "postgres")]
const SESSION_COLUMNS: &str =
    "id, user_id, created_at, last_used_at, expires_at, user_agent, ip";

#[cfg(feature = "postgres")]
const PAT_COLUMNS: &str =
    "id, name, user_id, created_at, scopes, last_used_at, expires_at";

/// Bounce a PG call off the tokio runtime onto a fresh OS thread.
/// Same rationale as `crate::storage::db::block_pg` — the sync
/// `postgres` crate panics if it tries to spin up its own runtime
/// inside an existing tokio context.
#[cfg(feature = "postgres")]
fn block_pg<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    std::thread::scope(|s| s.spawn(f).join().expect("pg auth dispatch thread panicked"))
}

#[cfg(feature = "postgres")]
impl UserStore for PgUserStore {
    // ── Users ───────────────────────────────────────────────────────────────

    fn create_user(&self, input: NewUser) -> Result<User> {
        block_pg(move || self.create_user_impl(input))
    }
    fn find_user_by_username(&self, username: &str) -> Result<Option<User>> {
        block_pg(|| self.find_user_by_username_impl(username))
    }
    fn find_user_by_id(&self, id: i64) -> Result<Option<User>> {
        block_pg(|| self.find_user_by_id_impl(id))
    }
    fn list_users(&self) -> Result<Vec<User>> {
        block_pg(|| self.list_users_impl())
    }
    fn delete_user(&self, id: i64) -> Result<bool> {
        block_pg(|| self.delete_user_impl(id))
    }
    fn count_users(&self) -> Result<i64> {
        block_pg(|| self.count_users_impl())
    }
    fn set_password(&self, user_id: i64, new_password: &str) -> Result<()> {
        block_pg(|| self.set_password_impl(user_id, new_password))
    }
    fn verify_password(&self, username: &str, password: &str) -> Result<Option<User>> {
        block_pg(|| self.verify_password_impl(username, password))
    }
    fn create_session(
        &self,
        user_id: i64,
        ttl_seconds: i64,
        user_agent: Option<&str>,
        ip: Option<&str>,
    ) -> Result<SessionToken> {
        block_pg(|| self.create_session_impl(user_id, ttl_seconds, user_agent, ip))
    }
    fn find_session_by_plaintext(&self, plaintext: &str) -> Result<Option<(Session, User)>> {
        block_pg(|| self.find_session_by_plaintext_impl(plaintext))
    }
    fn list_sessions_for_user(&self, user_id: i64) -> Result<Vec<Session>> {
        block_pg(|| self.list_sessions_for_user_impl(user_id))
    }
    fn revoke_session(&self, session_id: i64) -> Result<bool> {
        block_pg(|| self.revoke_session_impl(session_id))
    }
    fn touch_session(&self, session_id: i64) -> Result<()> {
        block_pg(|| self.touch_session_impl(session_id))
    }
    fn create_pat(
        &self,
        user_id: i64,
        name: &str,
        scopes: &[Scope],
        expires_at: Option<i64>,
    ) -> Result<(PersonalAccessToken, PatPlaintext)> {
        block_pg(|| self.create_pat_impl(user_id, name, scopes, expires_at))
    }
    fn find_pat_by_plaintext(
        &self,
        plaintext: &str,
    ) -> Result<Option<(PersonalAccessToken, User)>> {
        block_pg(|| self.find_pat_by_plaintext_impl(plaintext))
    }
    fn list_pats_for_user(&self, user_id: i64) -> Result<Vec<PersonalAccessToken>> {
        block_pg(|| self.list_pats_for_user_impl(user_id))
    }
    fn revoke_pat(&self, pat_id: i64) -> Result<bool> {
        block_pg(|| self.revoke_pat_impl(pat_id))
    }
    fn touch_pat(&self, pat_id: i64) -> Result<()> {
        block_pg(|| self.touch_pat_impl(pat_id))
    }
    fn get_repo_role(&self, repo: &str, user_id: i64) -> Result<Option<RepoRole>> {
        block_pg(|| self.get_repo_role_impl(repo, user_id))
    }
    fn set_repo_role(
        &self,
        repo: &str,
        user_id: i64,
        role: RepoRole,
        granted_by: Option<i64>,
    ) -> Result<()> {
        block_pg(|| self.set_repo_role_impl(repo, user_id, role, granted_by))
    }
    fn revoke_repo_role(&self, repo: &str, user_id: i64) -> Result<bool> {
        block_pg(|| self.revoke_repo_role_impl(repo, user_id))
    }
    fn list_repo_members(&self, repo: &str) -> Result<Vec<(User, RepoRole)>> {
        block_pg(|| self.list_repo_members_impl(repo))
    }
}

/// Inherent impls — the actual SQL bodies. Trait above is a thin
/// dispatch layer that ensures every entry hops onto a non-runtime
/// OS thread before touching `postgres::Client`.
#[cfg(feature = "postgres")]
impl PgUserStore {
    fn create_user_impl(&self, input: NewUser) -> Result<User> {
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
        let mut conn = self.conn()?;
        let now_ts = now();
        let admin_flag: i32 = if input.is_server_admin { 1 } else { 0 };
        let row = conn
            .query_one(
                "INSERT INTO users
                    (username, email, display_name, password_hash, is_server_admin, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 RETURNING id",
                &[
                    &input.username,
                    &input.email,
                    &display_name,
                    &pw_hash,
                    &admin_flag,
                    &now_ts,
                ],
            )
            .with_context(|| format!("insert user {}", input.username))?;
        let id: i64 = row.get(0);
        Ok(User {
            id,
            username: input.username,
            email: input.email,
            display_name,
            is_server_admin: input.is_server_admin,
            created_at: now_ts,
            last_login_at: None,
        })
    }

    fn find_user_by_username_impl(&self, username: &str) -> Result<Option<User>> {
        let mut conn = self.conn()?;
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE username = $1");
        let row = conn.query_opt(&sql, &[&username])?;
        Ok(row.as_ref().map(map_user_row))
    }

    fn find_user_by_id_impl(&self, id: i64) -> Result<Option<User>> {
        let mut conn = self.conn()?;
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE id = $1");
        let row = conn.query_opt(&sql, &[&id])?;
        Ok(row.as_ref().map(map_user_row))
    }

    fn list_users_impl(&self) -> Result<Vec<User>> {
        let mut conn = self.conn()?;
        let sql = format!("SELECT {USER_COLUMNS} FROM users ORDER BY username");
        let rows = conn.query(&sql, &[])?;
        Ok(rows.iter().map(map_user_row).collect())
    }

    fn delete_user_impl(&self, id: i64) -> Result<bool> {
        let mut conn = self.conn()?;
        // FK ON DELETE CASCADE handles sessions, PATs, repo_acls.
        let n = conn.execute("DELETE FROM users WHERE id = $1", &[&id])?;
        Ok(n > 0)
    }

    fn count_users_impl(&self) -> Result<i64> {
        let mut conn = self.conn()?;
        let row = conn.query_one("SELECT COUNT(*) FROM users", &[])?;
        Ok(row.get(0))
    }

    fn set_password_impl(&self, user_id: i64, new_password: &str) -> Result<()> {
        if new_password.is_empty() {
            bail!("password cannot be empty");
        }
        let hash = password::hash(new_password)?;
        let mut conn = self.conn()?;
        let n = conn.execute(
            "UPDATE users SET password_hash = $1 WHERE id = $2",
            &[&hash, &user_id],
        )?;
        if n == 0 {
            bail!("user {user_id} not found");
        }
        Ok(())
    }

    fn verify_password_impl(&self, username: &str, password: &str) -> Result<Option<User>> {
        let mut conn = self.conn()?;
        let sql = format!(
            "SELECT {USER_COLUMNS}, password_hash FROM users WHERE username = $1"
        );
        let row = conn.query_opt(&sql, &[&username])?;
        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };
        let user = map_user_row(&row);
        let hash: Option<String> = row.get("password_hash");
        let hash = match hash {
            Some(h) => h,
            None => return Ok(None),
        };
        if !password::verify(password, &hash)? {
            return Ok(None);
        }
        // Best-effort touch.
        let _ = conn.execute(
            "UPDATE users SET last_login_at = $1 WHERE id = $2",
            &[&now(), &user.id],
        );
        Ok(Some(user))
    }

    // ── Sessions ────────────────────────────────────────────────────────────

    fn create_session_impl(
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
        let mut conn = self.conn()?;
        let now_ts = now();
        let expires_at = now_ts + ttl_seconds;
        let row = conn.query_one(
            "INSERT INTO sessions
                (token_hash, token_prefix, user_id, created_at, last_used_at, expires_at, user_agent, ip)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             RETURNING id",
            &[
                &token.hash,
                &token.prefix,
                &user_id,
                &now_ts,
                &now_ts,
                &expires_at,
                &user_agent,
                &ip,
            ],
        )?;
        let id: i64 = row.get(0);
        Ok(SessionToken {
            session: Session {
                id,
                user_id,
                created_at: now_ts,
                last_used_at: now_ts,
                expires_at,
                user_agent: user_agent.map(str::to_string),
                ip: ip.map(str::to_string),
            },
            plaintext: token.plaintext,
        })
    }

    fn find_session_by_plaintext_impl(
        &self,
        plaintext: &str,
    ) -> Result<Option<(Session, User)>> {
        let prefix = tokens::prefix_of(plaintext);
        let mut conn = self.conn()?;
        // Reach for both tables in one query — column aliases keep
        // the row deserializer dialect-free.
        let session_cols = SESSION_COLUMNS
            .split(", ")
            .map(|c| format!("s.{c} AS s_{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let user_cols = USER_COLUMNS
            .split(", ")
            .map(|c| format!("u.{c} AS u_{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {session_cols}, s.token_hash AS s_token_hash, {user_cols}
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_prefix = $1"
        );
        let rows = conn.query(&sql, &[&prefix])?;
        let now_ts = now();
        for row in &rows {
            let session = Session {
                id: row.get("s_id"),
                user_id: row.get("s_user_id"),
                created_at: row.get("s_created_at"),
                last_used_at: row.get("s_last_used_at"),
                expires_at: row.get("s_expires_at"),
                user_agent: row.get("s_user_agent"),
                ip: row.get("s_ip"),
            };
            if session.expires_at <= now_ts {
                continue;
            }
            let token_hash: String = row.get("s_token_hash");
            if password::verify(plaintext, &token_hash)? {
                let user = User {
                    id: row.get("u_id"),
                    username: row.get("u_username"),
                    email: row.get("u_email"),
                    display_name: row.get("u_display_name"),
                    is_server_admin: row.get::<_, i32>("u_is_server_admin") != 0,
                    created_at: row.get("u_created_at"),
                    last_login_at: row.get("u_last_login_at"),
                };
                return Ok(Some((session, user)));
            }
        }
        Ok(None)
    }

    fn list_sessions_for_user_impl(&self, user_id: i64) -> Result<Vec<Session>> {
        let mut conn = self.conn()?;
        let sql = format!(
            "SELECT {SESSION_COLUMNS} FROM sessions WHERE user_id = $1
             ORDER BY last_used_at DESC"
        );
        let rows = conn.query(&sql, &[&user_id])?;
        Ok(rows.iter().map(map_session_row).collect())
    }

    fn revoke_session_impl(&self, session_id: i64) -> Result<bool> {
        let mut conn = self.conn()?;
        let n = conn.execute("DELETE FROM sessions WHERE id = $1", &[&session_id])?;
        Ok(n > 0)
    }

    fn touch_session_impl(&self, session_id: i64) -> Result<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "UPDATE sessions SET last_used_at = $1 WHERE id = $2",
            &[&now(), &session_id],
        )?;
        Ok(())
    }

    // ── PATs ────────────────────────────────────────────────────────────────

    fn create_pat_impl(
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
        let mut conn = self.conn()?;
        let now_ts = now();
        let row = conn.query_one(
            "INSERT INTO personal_access_tokens
                (name, token_hash, token_prefix, user_id, scopes, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING id",
            &[
                &name,
                &token.hash,
                &token.prefix,
                &user_id,
                &scopes_str,
                &now_ts,
                &expires_at,
            ],
        )?;
        let id: i64 = row.get(0);
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

    fn find_pat_by_plaintext_impl(
        &self,
        plaintext: &str,
    ) -> Result<Option<(PersonalAccessToken, User)>> {
        let prefix = tokens::prefix_of(plaintext);
        let mut conn = self.conn()?;
        let pat_cols = PAT_COLUMNS
            .split(", ")
            .map(|c| format!("p.{c} AS p_{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let user_cols = USER_COLUMNS
            .split(", ")
            .map(|c| format!("u.{c} AS u_{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {pat_cols}, p.token_hash AS p_token_hash, {user_cols}
             FROM personal_access_tokens p JOIN users u ON u.id = p.user_id
             WHERE p.token_prefix = $1"
        );
        let rows = conn.query(&sql, &[&prefix])?;
        let now_ts = now();
        for row in &rows {
            let scopes_str: String = row.get("p_scopes");
            let pat = PersonalAccessToken {
                id: row.get("p_id"),
                name: row.get("p_name"),
                user_id: row.get("p_user_id"),
                scopes: tokens::parse_scopes(&scopes_str).context("decode pat scopes")?,
                created_at: row.get("p_created_at"),
                last_used_at: row.get("p_last_used_at"),
                expires_at: row.get("p_expires_at"),
            };
            if let Some(exp) = pat.expires_at {
                if exp <= now_ts {
                    continue;
                }
            }
            let token_hash: String = row.get("p_token_hash");
            if password::verify(plaintext, &token_hash)? {
                let user = User {
                    id: row.get("u_id"),
                    username: row.get("u_username"),
                    email: row.get("u_email"),
                    display_name: row.get("u_display_name"),
                    is_server_admin: row.get::<_, i32>("u_is_server_admin") != 0,
                    created_at: row.get("u_created_at"),
                    last_login_at: row.get("u_last_login_at"),
                };
                return Ok(Some((pat, user)));
            }
        }
        Ok(None)
    }

    fn list_pats_for_user_impl(&self, user_id: i64) -> Result<Vec<PersonalAccessToken>> {
        let mut conn = self.conn()?;
        let sql = format!(
            "SELECT {PAT_COLUMNS} FROM personal_access_tokens WHERE user_id = $1
             ORDER BY created_at DESC"
        );
        let rows = conn.query(&sql, &[&user_id])?;
        rows.iter().map(map_pat_row).collect()
    }

    fn revoke_pat_impl(&self, pat_id: i64) -> Result<bool> {
        let mut conn = self.conn()?;
        let n = conn.execute(
            "DELETE FROM personal_access_tokens WHERE id = $1",
            &[&pat_id],
        )?;
        Ok(n > 0)
    }

    fn touch_pat_impl(&self, pat_id: i64) -> Result<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "UPDATE personal_access_tokens SET last_used_at = $1 WHERE id = $2",
            &[&now(), &pat_id],
        )?;
        Ok(())
    }

    // ── ACLs ────────────────────────────────────────────────────────────────

    fn get_repo_role_impl(&self, repo: &str, user_id: i64) -> Result<Option<RepoRole>> {
        let mut conn = self.conn()?;
        let row = conn.query_opt(
            "SELECT role FROM repo_acls WHERE repo = $1 AND user_id = $2",
            &[&repo, &user_id],
        )?;
        match row {
            Some(r) => {
                let s: String = r.get(0);
                Ok(Some(RepoRole::parse(&s)?))
            }
            None => Ok(None),
        }
    }

    fn set_repo_role_impl(
        &self,
        repo: &str,
        user_id: i64,
        role: RepoRole,
        granted_by: Option<i64>,
    ) -> Result<()> {
        let mut conn = self.conn()?;
        conn.execute(
            "INSERT INTO repo_acls (repo, user_id, role, granted_at, granted_by)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (repo, user_id) DO UPDATE
                SET role = EXCLUDED.role,
                    granted_at = EXCLUDED.granted_at,
                    granted_by = EXCLUDED.granted_by",
            &[&repo, &user_id, &role.as_str(), &now(), &granted_by],
        )?;
        Ok(())
    }

    fn revoke_repo_role_impl(&self, repo: &str, user_id: i64) -> Result<bool> {
        let mut conn = self.conn()?;
        let n = conn.execute(
            "DELETE FROM repo_acls WHERE repo = $1 AND user_id = $2",
            &[&repo, &user_id],
        )?;
        Ok(n > 0)
    }

    fn list_repo_members_impl(&self, repo: &str) -> Result<Vec<(User, RepoRole)>> {
        let mut conn = self.conn()?;
        let sql = format!(
            "SELECT {USER_COLUMNS}, a.role AS acl_role
             FROM repo_acls a JOIN users u ON u.id = a.user_id
             WHERE a.repo = $1
             ORDER BY u.username"
        );
        let rows = conn.query(&sql, &[&repo])?;
        let mut out = Vec::new();
        for row in &rows {
            let user = map_user_row(row);
            let role_str: String = row.get("acl_role");
            out.push((user, RepoRole::parse(&role_str)?));
        }
        Ok(out)
    }
}

// Suppress dead-code warnings when the postgres feature is off — the
// type still exists to satisfy `pub use` re-exports below the module
// gate without forcing every consumer to feature-gate too.
#[cfg(not(feature = "postgres"))]
#[allow(dead_code)]
pub struct PgUserStore;

#[allow(unused_imports)]
pub use anyhow as _anyhow_unused;
