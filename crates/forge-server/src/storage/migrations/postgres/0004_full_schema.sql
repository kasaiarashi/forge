-- Phase 7g (full coverage) — every remaining table the SQLite path
-- creates inline at boot. Lifting them onto Postgres so the whole
-- gRPC + admin surface is replicated, not just the trait-covered
-- atomic-push core.
--
-- Conventions match the rest of the Postgres baseline:
-- - BIGSERIAL for synthetic ids (mirrors SQLite's INTEGER PRIMARY
--   KEY AUTOINCREMENT).
-- - BIGINT epoch seconds for timestamps (no TIMESTAMPTZ until the
--   call sites pass chrono types).
-- - INTEGER 0/1 for booleans (matches SQLite; keeps trait return
--   types backend-agnostic).
-- - BYTEA for hashes / nonces / ciphertexts.

-- ── Issues / PRs / comments ────────────────────────────────────────

CREATE TABLE IF NOT EXISTS issues (
    id            BIGSERIAL PRIMARY KEY,
    repo          TEXT      NOT NULL,
    title         TEXT      NOT NULL,
    body          TEXT      NOT NULL DEFAULT '',
    author        TEXT      NOT NULL,
    status        TEXT      NOT NULL DEFAULT 'open',
    labels        TEXT      NOT NULL DEFAULT '',
    assignee      TEXT      NOT NULL DEFAULT '',
    created_at    BIGINT    NOT NULL,
    updated_at    BIGINT    NOT NULL,
    comment_count INTEGER   NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_issues_repo_status ON issues(repo, status);

CREATE TABLE IF NOT EXISTS pull_requests (
    id            BIGSERIAL PRIMARY KEY,
    repo          TEXT      NOT NULL,
    title         TEXT      NOT NULL,
    body          TEXT      NOT NULL DEFAULT '',
    author        TEXT      NOT NULL,
    status        TEXT      NOT NULL DEFAULT 'open',
    source_branch TEXT      NOT NULL,
    target_branch TEXT      NOT NULL DEFAULT 'main',
    labels        TEXT      NOT NULL DEFAULT '',
    assignee      TEXT      NOT NULL DEFAULT '',
    created_at    BIGINT    NOT NULL,
    updated_at    BIGINT    NOT NULL,
    comment_count INTEGER   NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_pull_requests_repo_status ON pull_requests(repo, status);

CREATE TABLE IF NOT EXISTS comments (
    id         BIGSERIAL PRIMARY KEY,
    repo       TEXT      NOT NULL,
    issue_id   BIGINT    NOT NULL,
    kind       TEXT      NOT NULL DEFAULT 'issue'
        CHECK (kind IN ('issue', 'pull_request')),
    author     TEXT      NOT NULL,
    body       TEXT      NOT NULL DEFAULT '',
    created_at BIGINT    NOT NULL,
    updated_at BIGINT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_comments_issue ON comments(repo, issue_id, kind);

-- ── Actions: workflows / runs / steps / artifacts / releases ──────

CREATE TABLE IF NOT EXISTS workflows (
    id         BIGSERIAL PRIMARY KEY,
    repo       TEXT      NOT NULL,
    name       TEXT      NOT NULL,
    yaml       TEXT      NOT NULL,
    enabled    INTEGER   NOT NULL DEFAULT 1,
    created_at BIGINT    NOT NULL,
    updated_at BIGINT    NOT NULL,
    UNIQUE (repo, name)
);

CREATE TABLE IF NOT EXISTS workflow_runs (
    id           BIGSERIAL PRIMARY KEY,
    repo         TEXT      NOT NULL,
    workflow_id  BIGINT    NOT NULL,
    trigger      TEXT      NOT NULL,
    trigger_ref  TEXT      NOT NULL DEFAULT '',
    commit_hash  TEXT      NOT NULL DEFAULT '',
    status       TEXT      NOT NULL DEFAULT 'queued',
    started_at   BIGINT,
    finished_at  BIGINT,
    created_at   BIGINT    NOT NULL,
    triggered_by TEXT      NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_workflow_runs_status     ON workflow_runs(status);
CREATE INDEX IF NOT EXISTS idx_workflow_runs_repo       ON workflow_runs(repo, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_workflow_runs_workflow   ON workflow_runs(workflow_id);

CREATE TABLE IF NOT EXISTS workflow_steps (
    id          BIGSERIAL PRIMARY KEY,
    run_id      BIGINT    NOT NULL,
    job_name    TEXT      NOT NULL,
    step_index  INTEGER   NOT NULL,
    name        TEXT      NOT NULL,
    status      TEXT      NOT NULL DEFAULT 'pending',
    exit_code   INTEGER,
    log         TEXT      NOT NULL DEFAULT '',
    started_at  BIGINT,
    finished_at BIGINT
);
CREATE INDEX IF NOT EXISTS idx_workflow_steps_run ON workflow_steps(run_id);

CREATE TABLE IF NOT EXISTS artifacts (
    id         BIGSERIAL PRIMARY KEY,
    run_id     BIGINT    NOT NULL,
    name       TEXT      NOT NULL,
    path       TEXT      NOT NULL,
    size_bytes BIGINT    NOT NULL DEFAULT 0,
    created_at BIGINT    NOT NULL,
    UNIQUE (run_id, name)
);
CREATE INDEX IF NOT EXISTS idx_artifacts_run ON artifacts(run_id);

CREATE TABLE IF NOT EXISTS releases (
    id         BIGSERIAL PRIMARY KEY,
    repo       TEXT      NOT NULL,
    run_id     BIGINT,
    tag        TEXT      NOT NULL,
    name       TEXT      NOT NULL,
    created_at BIGINT    NOT NULL,
    UNIQUE (repo, tag)
);

CREATE TABLE IF NOT EXISTS release_artifacts (
    release_id  BIGINT NOT NULL,
    artifact_id BIGINT NOT NULL,
    PRIMARY KEY (release_id, artifact_id)
);

-- ── Agents + run claims ───────────────────────────────────────────

CREATE TABLE IF NOT EXISTS agents (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT      NOT NULL UNIQUE,
    token_hash  TEXT      NOT NULL,
    labels_json TEXT      NOT NULL DEFAULT '[]',
    version     TEXT      NOT NULL DEFAULT '',
    os          TEXT      NOT NULL DEFAULT '',
    last_seen   BIGINT,
    created_at  BIGINT    NOT NULL
);

-- run_claims has no FK to workflow_runs because the run row may be
-- queried via different code paths than the claim. SQLite ships it
-- the same way; the cascade is handled by application logic when a
-- run finishes.
CREATE TABLE IF NOT EXISTS run_claims (
    run_id     BIGINT PRIMARY KEY,
    agent_id   BIGINT,
    claimed_at BIGINT NOT NULL
);

-- ── Secrets ────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS secrets (
    id         BIGSERIAL PRIMARY KEY,
    repo       TEXT      NOT NULL,
    key        TEXT      NOT NULL,
    nonce      BYTEA     NOT NULL,
    ciphertext BYTEA     NOT NULL,
    created_at BIGINT    NOT NULL,
    updated_at BIGINT    NOT NULL,
    UNIQUE (repo, key)
);
