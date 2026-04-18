-- Phase 7g — Postgres auth surface (users, sessions, PATs, ACLs).
--
-- Mirrors the SQLite schema captured by `MetadataDb::open` so
-- `PgUserStore` has parity with `SqliteUserStore`. Timestamps stay
-- BIGINT epoch seconds for cross-backend consistency. Booleans use
-- INTEGER (0/1) for the same reason — SQLite has no native bool, and
-- forcing the trait return type to be `bool` either way keeps every
-- caller dialect-free.

CREATE TABLE IF NOT EXISTS users (
    id              BIGSERIAL PRIMARY KEY,
    username        TEXT      NOT NULL UNIQUE,
    email           TEXT      NOT NULL UNIQUE,
    display_name    TEXT      NOT NULL,
    password_hash   TEXT,
    is_server_admin INTEGER   NOT NULL DEFAULT 0,
    created_at      BIGINT    NOT NULL,
    last_login_at   BIGINT
);

CREATE TABLE IF NOT EXISTS sessions (
    id           BIGSERIAL PRIMARY KEY,
    token_hash   TEXT      NOT NULL UNIQUE,
    token_prefix TEXT      NOT NULL,
    user_id      BIGINT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at   BIGINT    NOT NULL,
    last_used_at BIGINT    NOT NULL,
    expires_at   BIGINT    NOT NULL,
    user_agent   TEXT,
    ip           TEXT
);
CREATE INDEX IF NOT EXISTS idx_sessions_user   ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_prefix ON sessions(token_prefix);

CREATE TABLE IF NOT EXISTS personal_access_tokens (
    id           BIGSERIAL PRIMARY KEY,
    name         TEXT      NOT NULL,
    token_hash   TEXT      NOT NULL UNIQUE,
    token_prefix TEXT      NOT NULL,
    user_id      BIGINT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    scopes       TEXT      NOT NULL,
    created_at   BIGINT    NOT NULL,
    last_used_at BIGINT,
    expires_at   BIGINT
);
CREATE INDEX IF NOT EXISTS idx_pats_user   ON personal_access_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_pats_prefix ON personal_access_tokens(token_prefix);

CREATE TABLE IF NOT EXISTS repo_acls (
    repo       TEXT    NOT NULL,
    user_id    BIGINT  NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role       TEXT    NOT NULL CHECK (role IN ('read','write','admin')),
    granted_at BIGINT  NOT NULL,
    granted_by BIGINT  REFERENCES users(id),
    PRIMARY KEY (repo, user_id)
);
CREATE INDEX IF NOT EXISTS idx_repo_acls_user ON repo_acls(user_id);
