-- Phase 3b.5 — durable drain queue for S3-backed repo lifecycle ops.
-- Postgres mirror of the SQLite 0003 migration.
--
-- Same shape: append-only queue, visibility-timeout claim via
-- `not_before`. Postgres gets FOR UPDATE SKIP LOCKED on the claim
-- SELECT so multiple drain workers don't collide.

CREATE TABLE IF NOT EXISTS pending_repo_ops (
    id          BIGSERIAL PRIMARY KEY,
    op_type     TEXT      NOT NULL
        CHECK (op_type IN ('rename', 'delete')),
    repo        TEXT      NOT NULL,
    new_repo    TEXT,
    created_at  BIGINT    NOT NULL,
    not_before  BIGINT    NOT NULL DEFAULT 0,
    attempts    INTEGER   NOT NULL DEFAULT 0,
    last_error  TEXT
);

CREATE INDEX IF NOT EXISTS idx_pending_repo_ops_not_before
    ON pending_repo_ops (not_before);
