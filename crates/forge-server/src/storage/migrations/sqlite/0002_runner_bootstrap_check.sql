-- Phase 2b.1 migration-runner self-check.
--
-- This migration exists solely to prove the runner wires end-to-end:
-- servers running Phase 2a (no runner) will apply this on first boot
-- after upgrading, advancing schema_version from 1 to 2 and leaving
-- a single `runner_check` row so an operator can confirm the runner
-- fired at least once.
--
-- The row is never written to again. Future migrations replace this
-- file with real DDL.

CREATE TABLE IF NOT EXISTS schema_runner_check (
    id          INTEGER PRIMARY KEY,
    note        TEXT    NOT NULL,
    applied_at  INTEGER NOT NULL
);

INSERT INTO schema_runner_check (id, note, applied_at)
VALUES (1, 'runner bootstrapped in phase 2b.1', strftime('%s','now'))
ON CONFLICT(id) DO NOTHING;
