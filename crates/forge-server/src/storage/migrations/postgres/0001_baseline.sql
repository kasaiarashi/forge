-- Postgres baseline for the Phase-1 atomic-push surface.
--
-- Mirrors the SQLite schema captured by MetadataDb::open as of
-- Phase 2a: repos + refs + locks + upload_sessions + session_objects
-- + schema_version. Auth/issues/PRs/workflows/actions/agents live on
-- the concrete SQLite MetadataDb and are intentionally NOT included
-- here — Phase 2b.2 covers only the atomic-push trait surface.
--
-- Timestamps are kept as BIGINT epoch seconds (matching SQLite). This
-- avoids type-dispatch cruft in the trait impl; TIMESTAMPTZ is a
-- later migration once the code paths are ready for it.
--
-- Applied inside a single BEGIN/COMMIT by the migration runner so a
-- crash mid-apply leaves the DB on the previous revision.

CREATE TABLE IF NOT EXISTS repos (
    name            TEXT    PRIMARY KEY,
    description     TEXT    NOT NULL DEFAULT '',
    created_at      BIGINT  NOT NULL,
    visibility      TEXT    NOT NULL DEFAULT 'private'
        CHECK (visibility IN ('private', 'public')),
    default_branch  TEXT    NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS refs (
    repo  TEXT  NOT NULL REFERENCES repos(name) ON DELETE CASCADE,
    name  TEXT  NOT NULL,
    hash  BYTEA NOT NULL,
    PRIMARY KEY (repo, name)
);

CREATE TABLE IF NOT EXISTS locks (
    repo          TEXT    NOT NULL REFERENCES repos(name) ON DELETE CASCADE,
    path          TEXT    NOT NULL,
    owner         TEXT    NOT NULL,
    workspace_id  TEXT    NOT NULL,
    created_at    BIGINT  NOT NULL,
    reason        TEXT,
    PRIMARY KEY (repo, path)
);

CREATE TABLE IF NOT EXISTS upload_sessions (
    id            TEXT     PRIMARY KEY,
    repo          TEXT     NOT NULL,
    user_id       BIGINT,
    state         TEXT     NOT NULL
        CHECK (state IN ('uploading', 'committed', 'failed', 'abandoned'))
        DEFAULT 'uploading',
    created_at    BIGINT   NOT NULL,
    expires_at    BIGINT   NOT NULL,
    committed_at  BIGINT,
    result_json   TEXT,
    failure       TEXT
);
CREATE INDEX IF NOT EXISTS idx_upload_sessions_state
    ON upload_sessions (state, expires_at);

CREATE TABLE IF NOT EXISTS session_objects (
    session_id  TEXT     NOT NULL
        REFERENCES upload_sessions(id) ON DELETE CASCADE,
    hash        BYTEA    NOT NULL,
    size        BIGINT   NOT NULL,
    PRIMARY KEY (session_id, hash)
);
CREATE INDEX IF NOT EXISTS idx_session_objects_session
    ON session_objects (session_id);

CREATE TABLE IF NOT EXISTS schema_version (
    version     BIGINT  PRIMARY KEY,
    name        TEXT    NOT NULL,
    applied_at  BIGINT  NOT NULL
);
