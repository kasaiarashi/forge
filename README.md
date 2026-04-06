# Forge

A version control system built in Rust, purpose-built for Unreal Engine game development.

Forge treats large binary assets (.uasset, .umap, .uexp, .ubulk) as first-class citizens, provides Perforce-style file locking, and offers a simple CLI designed for game developers.

## Features

- **Binary-first** — Content-defined chunking (FastCDC) and deduplication for large assets
- **File locking** — Server-enforced exclusive locks prevent merge conflicts on binary files
- **Fast** — BLAKE3 hashing, zstd compression, parallel streaming transfers
- **Simple CLI** — Intuitive commands, no Git jargon
- **UE integration** — Source control plugin for the Unreal Editor

## Architecture

```
forge/
├── crates/
│   ├── forge-core/       # Core library: hashing, chunking, objects, storage
│   ├── forge-cli/        # CLI binary (the `forge` command)
│   ├── forge-server/     # gRPC server for remote operations
│   ├── forge-proto/      # Protobuf/gRPC definitions
│   └── forge-ignore/     # .forgeignore pattern matching
├── proto/                # Protocol buffer definitions
└── plugin/               # Unreal Engine source control plugin
```

## Commands

| Command | Description |
|---------|-------------|
| `forge init` | Initialize a new workspace |
| `forge add <paths>` | Stage files |
| `forge snapshot -m "msg"` | Create a snapshot (commit) |
| `forge status` | Show working directory status |
| `forge diff` | Show changes |
| `forge log` | Show snapshot history |
| `forge push` / `pull` | Sync with server |
| `forge clone <url>` | Clone a remote project |
| `forge lock <file>` | Lock a file for exclusive editing |
| `forge unlock <file>` | Release a lock |
| `forge branch [name]` | List or create branches |
| `forge switch <name>` | Switch branches |

## Building

```bash
cargo build --release
```

The `forge` binary will be at `target/release/forge`.

## Tech Stack

- **BLAKE3** for content-addressable hashing
- **FastCDC** for content-defined chunking of large files
- **zstd** for compression
- **gRPC (tonic)** for client-server protocol
- **SQLite** for server-side metadata (refs, locks)

## License

MIT OR Apache-2.0
