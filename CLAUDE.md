# Forge VCS

Forge is a version control system built in Rust for Unreal Engine game development. Binary-first with file locking, designed as a git-compatible alternative for game teams.

## Architecture

- **forge-core** — Core library: BLAKE3 hashing, FastCDC chunking, zstd compression, index, workspace, object store, diff
- **forge-cli** — CLI client (`forge` binary) with git-compatible commands
- **forge-server** — gRPC server for remote operations (push/pull/locks), SQLite metadata
- **forge-proto** — Protobuf definitions for gRPC
- **forge-ignore** — .forgeignore pattern matching
- **forge-web** — Web UI server + React frontend

## UE Plugin

- Located at `plugin/ForgeSourceControl/Plugins/ForgeSourceControl/`
- UE project (ForgeVCS) at `plugin/ForgeSourceControl/`
- Implements `ISourceControlProvider` via `IModularFeatures`
- Shells out to `forge` CLI with `--json` flag
- Targets UE 5.7

## Build

```sh
cargo build --release
```

Always use `--release` profile.

## Key Conventions

- Staged deletions use `ForgeHash::ZERO` as sentinel
- Objects stored at `.forge/objects/<first2hex>/<rest>`, zstd compressed
- Branches at `.forge/refs/heads/<name>`, tags at `.forge/refs/tags/<name>`
- Index is bincode-serialized at `.forge/index`
- Config at `.forge/config.json`

## Commit Style

Short summary line describing the "what" and "why". No co-author lines.
