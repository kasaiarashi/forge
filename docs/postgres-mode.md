# Postgres mode

Phase 7g shipped first-class Postgres support so a forge-server
deployment can fan auth + core VCS state across multiple instances
via Postgres streaming replication. SQLite remains the default and
single-host happy path; Postgres is opt-in.

## What works on Postgres

**Everything.** Phase 7g (full coverage) lifted every module's
schema + queries onto `PgMetadataBackend`. When `[database]
backend = "postgres"` is selected, the SQLite pool is never opened
— there is no `forge.db` file under `<base_path>`. Every gRPC
handler, admin CLI, and background sweeper goes straight to
Postgres.

Covered surfaces:

- Repos / refs / locks
- Upload sessions + atomic push + pending repo ops
- Health probes (`/healthz`, `/readyz`)
- `/metrics` counters
- Users / sessions / PATs / repo ACLs (`PgUserStore`)
- Issues / pull requests / comments
- Workflows / runs / steps
- Artifacts metadata / releases / retention
- Agents (registration + claims)
- Secrets (encrypted with AES-GCM, ciphertext lives in Postgres)
- Default branch per repo

## Bootstrap with Docker (recommended for self-hosting)

```sh
forge-server postgres up
```

This:
1. Pulls `postgres:16` if missing.
2. Generates a random password under
   `<base_path>/postgres/credentials.json` (mode 0600).
3. Bind-mounts `<base_path>/postgres/data/` into
   `/var/lib/postgresql/data` inside the container.
4. Waits for `pg_isready`.
5. Rewrites `forge-server.toml`'s `[database]` block to `backend
   = "postgres"` + the new connection URL.

After that, `forge-server serve` runs against Postgres. `forge-server
postgres status` reports container state; `forge-server postgres
down [--rm]` stops it.

The data directory living next to `forge-data` is deliberate —
`tar cz <base_path>` captures the database alongside objects so the
deployment stays transferable. Move the tarball, restore it, and
the new host sees the full state.

## Bootstrap with an external Postgres

Skip the Docker subcommand and edit `forge-server.toml` by hand:

```toml
[database]
backend = "postgres"
url = "postgres://forge:hunter2@db.studio.example:5432/forge"
max_connections = 32
```

Then run `forge-server migrate` to create the schema, `forge-server
user add --admin <name>` to create the first admin, and `forge-server
serve` to start the server.

## Installer integration

Both Linux and macOS installers honour `FORGE_USE_POSTGRES=1`:

```sh
sudo FORGE_USE_POSTGRES=1 ./install.sh
```

When set and Docker is on PATH, the installer runs `forge-server
postgres up` after laying down the config + binaries, then
restarts the systemd / launchd unit so it picks up the new
`[database]` block.

The Windows InnoSetup installer exposes the same option as a task
on the components page ("Run a containerised Postgres backend").

## Why not full async?

The `postgres` crate is a sync facade over `tokio_postgres`. Calling
its sync API from inside an existing tokio runtime (i.e. the gRPC
server) panics with "Cannot start a runtime from within a runtime".

The `MetadataBackend` trait stays sync, and every Postgres-bound
trait method bounces onto a fresh OS thread via
`crate::storage::db::block_pg` (a thin `std::thread::scope` wrapper)
before touching the postgres client. This adds a thread-spawn per
query — acceptable for the trait surface where queries are
millisecond-scale.

A future async refactor would let the server use `tokio_postgres`
directly and skip the thread-bounce overhead. For Phase 7g we
chose the smaller change that lets all 75+ existing
`self.db.<method>()` call sites stay synchronous.

## Replication for HA

`forge-server` doesn't ship a replication layer; use Postgres'
built-in physical or logical replication. The recommended setup:

- Primary writes via `forge-server serve` on one host
- Hot-standby Postgres on N nodes via `recovery.conf` /
  `postgresql.conf` `standby_mode`
- Edge `forge-server serve --read-only` on each standby (Phase 7e)

When the primary fails, promote a standby, repoint
`[database] url`, and bounce the edges. No special tooling needed
on forge-server's side beyond what's documented above.
