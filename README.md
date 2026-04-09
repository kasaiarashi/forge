# Forge

<p align="center">
  <img src="docs/branding/forge-logo.svg" width="320" alt="Forge VCS" />
</p>

A version control system built in Rust, purpose-built for Unreal Engine game development.

Forge treats large binary assets (.uasset, .umap, .uexp, .ubulk) as first-class citizens, provides Perforce-style file locking, and offers a simple CLI designed for game developers.

## Quick Setup

Install the Forge server on the host that will store your repos. Clients connect to it over gRPC on port `9876` by default.

### Linux

```bash
curl -fsSL https://raw.githubusercontent.com/kasaiarashi/forge/master/installers/linux/install.sh | sudo bash
```

The script ([`installers/linux/install.sh`](installers/linux/install.sh)) downloads the latest release tarball, extracts it, drops `forge-server` and `forge-web` into `/usr/local/bin`, the web UI into `/usr/local/share/forge/ui`, configs into `/etc/forge/`, and data into `/var/lib/forge/`. Override any of these via `PREFIX=`, `CONFIG_DIR=`, `DATA_DIR=`, or pin a specific version with `FORGE_VERSION=v0.1.0`.

### macOS

```bash
brew install kasaiarashi/forge/forge-server
brew services start forge-server
```

The tap lives at [kasaiarashi/homebrew-forge](https://github.com/kasaiarashi/homebrew-forge). Configs land in `$(brew --prefix)/etc/forge/` and data in `$(brew --prefix)/var/forge/`.

### Windows

Download and run the server installer:

[**ForgeServer-Windows-x64-Setup.exe**](https://github.com/kasaiarashi/forge/releases/download/v0.1.0/ForgeServer-Windows-x64-Setup.exe)

The installer registers Forge as a Windows service, drops a default `forge-server.toml`, and starts the service automatically.

After install on any platform, watch the server log for the **TLS CA fingerprint** line — clients verify it on first `forge login` to pin the self-signed CA (TOFU).

## Why Forge?

Game projects break Git. Game projects break Perforce. Forge was built because neither tool was designed for the reality of modern game development: repos with tens of thousands of multi-gigabyte binary assets, artists and programmers on the same team, and builds that can't wait for version control to catch up.

Forge is not a Git wrapper, a Git fork, or a Git clone. It is a new version control system with its own object model, storage engine, and network protocol, designed from scratch around the problems that game teams actually hit.

### What's wrong with Git?

Git was built for the Linux kernel — millions of small text files. It falls apart on game projects:

- **Binary files are an afterthought.** Git stores full copies of every version of every binary. A 500 MB .umap that changes 100 times costs 50 GB of history. Git LFS bolts on a pointer-file workaround, but it breaks `blame`, `diff`, `bisect`, and offline workflows. LFS is a band-aid on a design that was never meant to handle binaries.
- **No file locking.** Two artists edit the same Blueprint and somebody's work gets thrown away. Git's merge model assumes text, and binary merges are not possible. Git LFS adds advisory locks, but they're client-side and unenforceable.
- **Clones are brutal.** `git clone` on a 200 GB game repo can take hours. Shallow clones help, but break half the commands. Partial clones are experimental and poorly supported by most hosts.
- **Repack and GC are expensive.** Git periodically repacks objects using delta compression tuned for text. On binary-heavy repos, repack can run for hours, peg CPU and memory, and produce marginal savings because binary deltas compress poorly.

### What's wrong with Perforce?

Perforce handles large files, but it was designed in the 90s and it shows:

- **Centralized and fragile.** One server, one point of failure. If the Perforce server goes down, nobody can commit, branch, or diff. Remote teams suffer high-latency operations on every file open.
- **Workspace mapping is painful.** Perforce requires explicit client specs that map depot paths to local paths. Setting up a workspace for a new team member involves arcane `p4 client` configuration that trips up even experienced developers.
- **Expensive at scale.** Perforce licenses are per-seat and per-server. Large game studios pay significant sums for the privilege of using a tool that predates modern networking.
- **Branching is heavyweight.** A Perforce "branch" copies files on the server. Creating a feature branch on a 200 GB repo can take minutes and doubles storage. Teams avoid branching, which leads to worse collaboration.
- **Tooling lock-in.** P4V is dated. Third-party integrations are limited compared to the Git ecosystem. CI/CD pipelines, code review tools, and automation all assume Git.

### How Forge is different

Forge is not Git with better binary support or Perforce with distributed commits. It's a different design built on different assumptions:

| | Git | Perforce | Forge |
|---|---|---|---|
| **Binary storage** | Full copies (or LFS pointer files) | Full copies, server-side | Content-defined chunking (FastCDC) with deduplication — a 500 MB file that changes slightly stores only the changed chunks |
| **Compression** | zlib, delta chains tuned for text | Server-side, opaque | zstd at every layer — 3-5x faster than zlib at similar ratios |
| **Hashing** | SHA-1 (deprecated, collision-vulnerable) | MD5 (server-side) | BLAKE3 — cryptographically secure and faster than MD5 on modern hardware |
| **Locking** | None (LFS adds advisory locks) | Server-enforced | Server-enforced exclusive locks with auto-lock patterns (e.g. `*.uasset`) |
| **Branching** | Cheap (pointer move) | Expensive (file copies) | Cheap (pointer move, like Git) |
| **Offline work** | Full repo clone required | Not possible | Full local history, commit and branch offline |
| **Large file performance** | Degrades with size | Good with server bandwidth | Parallel chunked transfers, deduplicated across files and versions |

### The familiar part: Git-compatible command syntax

Forge deliberately uses Git's command names and flags. If you know Git, you already know Forge:

```bash
forge init                    # like git init
forge add .                   # like git add
forge commit -m "message"     # like git commit
forge status                  # like git status
forge log --oneline           # like git log
forge branch feature/foo      # like git branch
forge switch feature/foo      # like git switch
forge merge feature/foo       # like git merge
forge diff --staged           # like git diff
forge stash / forge stash pop # like git stash
forge push / forge pull       # like git push/pull
forge reset --hard <commit>   # like git reset
forge clone <url>             # like git clone
```

The command surface is intentionally familiar so developers don't have to learn a new mental model. The difference is under the hood: every command is implemented from scratch with binary assets, large repos, and game team workflows in mind. There's no shell-out to Git, no LFS subprocess, no translation layer.

**What Forge adds that Git doesn't have:**
- `forge lock <file>` / `forge unlock <file>` — Server-enforced exclusive file locking
- `forge asset-info <file>` — UE asset metadata inspection (.uasset/.umap)
- `forge gc` — Object store garbage collection with `--dry-run`
- Auto-lock patterns in config — Automatically lock `*.uasset`, `*.umap` on edit
- Native Unreal Editor plugin — Source control from within the editor, no external tools

## Features

- **Binary-first** — Content-defined chunking (FastCDC) and deduplication for large assets
- **File locking** — Server-enforced exclusive locks prevent merge conflicts on binary files
- **Fast** — BLAKE3 hashing, zstd compression, parallel streaming transfers
- **Simple CLI** — Git-compatible commands, zero learning curve for developers
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
| `forge commit -m "msg"` | Commit staged changes |
| `forge status` | Show working directory status |
| `forge diff` | Show changes |
| `forge log` | Show commit history (`--all` for all branches) |
| `forge push` / `pull` | Sync with server |
| `forge clone <url>` | Clone a remote project |
| `forge branch [name]` | List or create branches |
| `forge switch <name>` | Switch branches |
| `forge merge <branch>` | Merge a branch |
| `forge stash` / `pop` | Stash and restore working changes |
| `forge reset [--soft\|--hard]` | Reset HEAD to a commit |
| `forge lock <file>` | Lock a file for exclusive editing |
| `forge unlock <file>` | Release a lock |
| `forge gc` | Prune unreachable objects (`--dry-run` to preview) |
| `forge asset-info <file>` | Inspect UE asset metadata |

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

## Author

Krishna Teja ([@krishna18developer](mailto:krishna18developer@gmail.com))

## License

MIT
