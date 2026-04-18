-- Phase 3b.5 — durable drain queue for S3-backed repo lifecycle ops.
--
-- S3 has no atomic "rename prefix" or "delete prefix" primitive, so
-- rename_repo / delete_repo enqueue a row here and a background drain
-- task walks the bucket keyspace with CopyObject + DeleteObjects. Rows
-- survive server restarts so a kill -9 mid-drain resumes on next boot.
--
-- Visibility timeout pattern: the drain "claims" a row by bumping
-- `not_before` to now + timeout; if the drain crashes, the row becomes
-- eligible again once `not_before` passes. `attempts` doubles as a
-- retry counter so operators can spot a stuck op.

CREATE TABLE IF NOT EXISTS pending_repo_ops (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    op_type     TEXT    NOT NULL
        CHECK (op_type IN ('rename', 'delete')),
    repo        TEXT    NOT NULL,
    -- Destination name for 'rename'; NULL for 'delete'.
    new_repo    TEXT,
    created_at  INTEGER NOT NULL,
    -- Unix epoch seconds; row is eligible to claim when now >= not_before.
    not_before  INTEGER NOT NULL DEFAULT 0,
    attempts    INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT
);

CREATE INDEX IF NOT EXISTS idx_pending_repo_ops_not_before
    ON pending_repo_ops (not_before);
