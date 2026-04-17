// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Numbered schema migrations.
//!
//! At `MetadataDb::open`, the `schema_version` table is seeded at
//! revision 1 (the "baseline" — the schema captured by the inline
//! `CREATE TABLE` statements in [`crate::storage::db`]). Every
//! revision after that lives as a `.sql` file under the backend-
//! specific subdirectory here, embedded into the binary via
//! `include_str!` and listed in [`SQLITE_MIGRATIONS`].
//!
//! The runner is deliberately append-only: **never edit a migration
//! that has already shipped to a deployed server**. Fix-forward
//! migrations land as a new numbered revision.
//!
//! ## Adding a new migration (SQLite)
//!
//! 1. Pick the next free version number `N`.
//! 2. Drop the SQL into `sqlite/NNNN_<short_name>.sql` (e.g.
//!    `0002_add_integrations_table.sql`). Use `NNNN` padding so
//!    `ls`-sorted listings match version order.
//! 3. Add a new [`Migration`] entry at the bottom of
//!    [`SQLITE_MIGRATIONS`] — version, human-readable name, the SQL
//!    via `include_str!`. The runner applies pending migrations at
//!    every `open()` in ascending order inside a `BEGIN IMMEDIATE`
//!    transaction that also INSERTs the `schema_version` row, so a
//!    crashed migration leaves the DB on the previous revision.
//!
//! Postgres migrations ride under `postgres/` once the Phase 2b.2
//! backend lands; the runner picks the right list by backend at
//! startup.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// A single schema revision. `sql` is executed via
/// [`rusqlite::Connection::execute_batch`] so it may contain multiple
/// statements separated by `;`.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// Ascending, strictly greater than every prior entry.
    pub version: i64,
    /// Short, hyphen-or-underscore name for operator logs.
    pub name: &'static str,
    /// Full SQL body (may contain multiple statements).
    pub sql: &'static str,
}

/// SQLite migrations, in ascending version order.
///
/// **Revision 1 (the baseline) is applied implicitly by the
/// `create_*_tables()` methods in `MetadataDb::open`**, so this list
/// starts at revision 2. Leaving the baseline out of the list keeps
/// the bootstrap path identical across deployments that already ran
/// Phase 2a vs. fresh installs that got there via the baseline DDL.
pub const SQLITE_MIGRATIONS: &[Migration] = &[
    // Phase 2b.1 self-check: a no-op migration that proves the runner
    // wires correctly end-to-end. Future phases replace this with real
    // DDL (e.g. Phase 5 integrations table, changelists, typemap).
    Migration {
        version: 2,
        name: "runner_bootstrap_check",
        sql: include_str!("sqlite/0002_runner_bootstrap_check.sql"),
    },
];

/// Apply every migration in `list` whose version is strictly greater
/// than `current`. Each migration runs inside its own
/// `BEGIN IMMEDIATE` transaction alongside the matching
/// `schema_version` insert, so partial failure is impossible: either
/// the SQL committed and the revision row exists, or neither did.
///
/// Returns the number of migrations applied.
pub fn apply_pending(
    conn: &mut Connection,
    current: i64,
    list: &[Migration],
) -> Result<usize> {
    let mut applied = 0usize;
    // Callers pass lists in ascending order (enforced in debug builds),
    // but double-check here so a future maintainer can't silently
    // break ordering by a typo.
    for window in list.windows(2) {
        debug_assert!(
            window[0].version < window[1].version,
            "migration versions must strictly ascend: {} before {}",
            window[0].version,
            window[1].version,
        );
    }

    for m in list.iter().filter(|m| m.version > current) {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .with_context(|| format!("begin migration {} ({})", m.version, m.name))?;
        tx.execute_batch(m.sql)
            .with_context(|| format!("execute migration {} ({})", m.version, m.name))?;
        let now = chrono::Utc::now().timestamp();
        tx.execute(
            "INSERT INTO schema_version (version, name, applied_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![m.version, m.name, now],
        )
        .with_context(|| format!("record migration {} ({})", m.version, m.name))?;
        tx.commit()
            .with_context(|| format!("commit migration {} ({})", m.version, m.name))?;
        tracing::info!(
            version = m.version,
            name = m.name,
            "schema migration applied"
        );
        applied += 1;
    }

    Ok(applied)
}
