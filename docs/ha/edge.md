# Read-only edge replicas

Phase 7e ships a `--read-only` mode for `forge-server` so a single
primary can be flanked by N edge replicas that absorb pull / has /
ref-read traffic close to artists. The primary stays the sole writer
— pushes, lock acquires, and ref updates always land there — but
checkout and CI fetches never round-trip across the WAN.

## Architecture

```
                 ┌────────────────────────────────────────┐
                 │      Primary forge-server (writer)     │
                 │  - SQLite WAL (replicated by Litestream)│
                 │  - Object store (replicated to S3 / NFS)│
                 └──────────────┬─────────────────────────┘
                                │ Litestream → S3 WAL stream
                                │ rsync / `aws s3 sync` / hardlink farm
              ┌─────────────────┼─────────────────┐
              ▼                 ▼                 ▼
      ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
      │ Edge (read)  │  │ Edge (read)  │  │ Edge (read)  │
      │ --read-only  │  │ --read-only  │  │ --read-only  │
      └──────────────┘  └──────────────┘  └──────────────┘
            ▲                  ▲                  ▲
            └─── artists / CI agents pull from nearest edge ───┘
```

## Replication

The edge has no DB or object writes of its own; it is purely a
replica. Two streams keep it warm:

1. **Metadata** — Litestream replicates the primary's SQLite WAL to
   S3 (or any object store Litestream supports). Each edge restores
   from the same WAL stream and then tails it. See
   `docs/ha/litestream.yml.example` for the ready-to-paste config.

2. **Objects** — content-addressed and immutable, so any
   filesystem-level mirror works:
   - `rsync -a --delete primary:/var/lib/forge/objects/ /var/lib/forge/objects/`
     on a five-minute cron.
   - `aws s3 sync s3://forge-primary/objects/ s3://forge-edge/objects/`
     when both ends are S3-backed (`[objects] backend = "s3"`).
   - For LAN deployments, an NFS / SMB mount of the primary's
     objects directory works and skips replication entirely.

   Whichever you pick, point `[storage] base_path` at the local
   replicated copy. The edge process never writes there.

## Running an edge

```sh
forge-server serve \
    --read-only \
    --upstream-write-url https://forge.studio.example:50051
```

`--upstream-write-url` is the public address of the primary. The
edge surfaces that URL in the error message a write RPC sees, so
client tooling that supports edge-aware retry can transparently
re-route. Today's `forge-cli` does not yet read the hint — operators
should configure clients to point at the edge for reads and the
primary for writes manually until the smart-retry change ships.

## Operational notes

- **Lag**: Litestream is asynchronous. An edge can be seconds to a
  minute behind the primary. A push that just landed on the
  primary may briefly 404 from an edge until the WAL catches up.
  Document the bound (`max_replication_lag_seconds`) and surface
  it to artists if it matters.

- **Failover**: an edge is *not* a hot standby. It cannot be
  promoted to primary without first fully replaying the latest
  WAL — Litestream's `restore` subcommand does this, but the
  switchover is operator-driven, not automatic.

- **Auth**: the edge needs read access to the same DB the primary
  uses, so PATs / sessions / repo ACLs work transparently. New
  PATs minted on the primary become usable on every edge once the
  WAL replicates.

- **Lock subscriptions**: `StreamLockEvents` reads from the local
  DB. Because the WAL stream is async, the edge's "live" feed is
  delayed by the same window as the rest of the replica state.
  For studios that need millisecond-fresh lock events, point UE
  plugins at the primary for the lock stream while still pulling
  objects from the edge.

## What `--read-only` actually does

The flag installs a tower layer (`services::edge::ReadOnlyLayer`)
that inspects each gRPC method path and short-circuits writes
with `FailedPrecondition`. The full list of write paths lives in
`services/edge.rs`. Adding a new RPC requires categorising it in
that file — the tower layer is deliberately fail-closed: any
unrecognised path defaults to "allowed", so a missed entry will
let writes through.
