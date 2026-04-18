// Pull shared modules + helpers from the library crate. The binary's
// purpose is solely to wire up the clap parser → command dispatch;
// all state lives in `lib.rs`.
use forge_cli::commands;
use forge_cli::server_url_hint;
#[allow(unused_imports)]
use forge_cli::set_server_url_hint; // May be unused depending on features.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "forge",
    about = "Version control for Unreal Engine",
    version,
    // Keep in lockstep with commands::version::render_banner so
    // `forge --version` and `forge version`/`forge info` print the
    // same leading block. Clap prepends the command name ("forge ")
    // before the first line of `long_version`, so we deliberately
    // start with just the bare version number + metadata tail.
    long_version = concat!(
        env!("CARGO_PKG_VERSION"), "\n",
        "Copyright (c) 2026 Krishna Teja Mekala \u{2014} https://github.com/kasaiarashi/forge\n",
        "Licensed under BSL 1.1",
    ),
)]
struct Cli {
    /// Output in JSON format (for tooling/UE plugin).
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new forge workspace
    Init,

    /// Add files to the staging area
    Add {
        /// Files or directories to add
        paths: Vec<String>,
    },

    /// Commit current changes
    Commit {
        /// Commit message (optional with --amend; reuses prior message if omitted)
        #[arg(short, long)]
        message: Option<String>,

        /// Commit all changed files (skip explicit staging)
        #[arg(short, long)]
        all: bool,

        /// Replace the tip commit instead of creating a new one
        #[arg(long)]
        amend: bool,
    },

    /// Show working directory status
    Status,

    /// Show differences
    Diff {
        /// Compare against a specific commit
        #[arg(long)]
        commit: Option<String>,

        /// Show staged changes (index vs HEAD)
        #[arg(long)]
        staged: bool,

        /// Show only file change summary (insertions/deletions)
        #[arg(long)]
        stat: bool,

        /// Extract two versions to temp files (for external diff tools / UE editor)
        #[arg(long)]
        extract: bool,

        /// Disable the pager and write output directly to stdout
        #[arg(long)]
        no_pager: bool,

        /// Emit a per-class export count delta (diagnostic for large BP diffs)
        #[arg(long)]
        class_stats: bool,

        /// File paths to restrict diff to
        paths: Vec<String>,
    },

    /// Show commit history
    Log {
        /// Number of commits to show
        #[arg(short = 'n', long, default_value_t = 20)]
        count: u32,

        /// Show history for a specific file
        #[arg(long)]
        file: Option<String>,

        /// One commit per line (hash + message)
        #[arg(long)]
        oneline: bool,

        /// Show commits from all branches
        #[arg(long)]
        all: bool,

        /// Disable the pager and write output directly to stdout
        #[arg(long)]
        no_pager: bool,
    },

    /// Push commits to the server
    Push {
        /// Force push (overwrite remote ref even if diverged)
        #[arg(short, long)]
        force: bool,
        /// Optional remote name (git-compat; must match the configured remote when set)
        remote: Option<String>,
        /// Optional branch ref (git-compat; must match the current branch when set)
        branch: Option<String>,
    },

    /// Pull commits from the server
    Pull,

    /// Download remote branches and update remote-tracking refs (no checkout)
    Fetch {
        /// Branch to fetch (omit to fetch all remote branches)
        branch: Option<String>,
    },

    /// Clone a remote project
    Clone {
        /// Server URL
        url: String,

        /// Repository name on the server (defaults to "default")
        #[arg(long)]
        repo: Option<String>,

        /// Local directory (defaults to repo name)
        #[arg(long)]
        path: Option<String>,
    },

    /// Lock a file for exclusive editing
    Lock {
        /// File path to lock
        path: String,

        /// Reason for locking
        #[arg(short, long)]
        reason: Option<String>,
    },

    /// Unlock a file
    Unlock {
        /// File path to unlock
        path: String,

        /// Force unlock (admin)
        #[arg(long)]
        force: bool,
    },

    /// List active locks
    Locks,

    /// Unstage files (undo forge add)
    Unstage {
        /// Files or directories to unstage (use . for all)
        paths: Vec<String>,
    },

    /// Restore working tree files
    Restore {
        /// Unstage files (like git restore --staged)
        #[arg(long)]
        staged: bool,

        /// Restore from a specific commit or branch
        #[arg(long)]
        source: Option<String>,

        /// Files or directories to restore
        paths: Vec<String>,
    },

    /// Create or list branches
    Branch {
        /// Branch name (omit to list)
        name: Option<String>,

        /// Delete the branch
        #[arg(short, long)]
        delete: bool,

        /// List both local and remote-tracking branches
        #[arg(short = 'a', long)]
        all: bool,

        /// List only remote-tracking branches
        #[arg(short = 'r', long)]
        remotes: bool,
    },

    /// Switch to a branch (optionally creating it first)
    Switch {
        /// Branch name
        name: String,

        /// Create <name> at current HEAD before switching. Like
        /// `git switch -c <name>` or `git checkout -b <name>` — the
        /// canonical way to promote a detached HEAD into a branch you
        /// can commit onto.
        #[arg(short = 'c', long = "create")]
        create: bool,
    },

    /// Manage .forgeignore patterns
    Ignore {
        /// Patterns to add
        patterns: Vec<String>,
    },

    /// Manage remote servers
    Remote {
        /// Action: add, remove, rename, set-url (omit to list)
        action: Option<String>,

        /// Arguments for the action (e.g., name, url)
        args: Vec<String>,
    },

    /// View or set workspace configuration
    Config {
        /// Config key (e.g., workflow, user.name)
        key: Option<String>,

        /// Value to set
        value: Option<String>,
    },

    /// Create or list tags
    Tag {
        /// Tag name (omit to list)
        name: Option<String>,

        /// Commit hash (defaults to HEAD)
        #[arg(long)]
        commit: Option<String>,

        /// Delete the tag
        #[arg(short, long)]
        delete: bool,
    },

    /// Remove files from the working tree and index
    Rm {
        /// Files to remove
        paths: Vec<String>,

        /// Only remove from the index (keep file on disk)
        #[arg(long)]
        cached: bool,

        /// Recurse into directories: expands each directory path to every
        /// tracked file underneath it. Matches `git rm -r` semantics.
        #[arg(short, long)]
        recursive: bool,
    },

    /// Move or rename a file
    Mv {
        /// Source path
        source: String,

        /// Destination path
        dest: String,
    },

    /// Show commit details and diff
    Show {
        /// Commit hash (defaults to HEAD)
        commit: Option<String>,
    },

    /// Merge a branch into the current branch
    Merge {
        /// Branch to merge
        branch: String,
    },

    /// Apply changes from a specific commit
    #[command(name = "cherry-pick")]
    CherryPick {
        /// Commit to cherry-pick
        commit: String,
    },

    /// Checkout a branch or restore files
    Checkout {
        /// Branch name or commit hash
        target: Option<String>,
        /// File paths to restore (after --)
        #[arg(last = true)]
        paths: Vec<String>,
    },

    /// Remove untracked files from the working tree
    Clean {
        /// Force deletion (required to actually delete)
        #[arg(short, long)]
        force: bool,

        /// Also remove untracked directories
        #[arg(short = 'd', long)]
        directories: bool,
    },

    /// Reset HEAD to a specific commit
    Reset {
        /// Target commit (defaults to HEAD)
        commit: Option<String>,
        /// Soft reset (move HEAD only)
        #[arg(long)]
        soft: bool,
        /// Hard reset (restore working tree)
        #[arg(long)]
        hard: bool,
    },

    /// Stash working directory changes
    Stash {
        /// Action: push (default), pop, apply, list, drop
        action: Option<String>,
        /// Stash message
        #[arg(short, long)]
        message: Option<String>,
    },

    /// Revert a commit by creating a new inverse commit
    Revert {
        /// Commit to revert
        commit: String,
    },

    /// Show UE asset metadata (.uasset/.umap)
    AssetInfo {
        /// Path to the asset file
        path: String,
    },

    /// Garbage collect unreachable objects
    Gc {
        /// Show what would be pruned without deleting
        #[arg(long)]
        dry_run: bool,
    },

    /// Check for updates and self-update the forge CLI
    Update {
        /// Only check for updates without installing
        #[arg(long)]
        check: bool,

        /// Force re-download even if already on the latest version
        #[arg(short, long)]
        force: bool,
    },

    /// Authenticate against a forge server and store the credential
    Login {
        /// Server URL (defaults to current workspace's remote)
        #[arg(long)]
        server: Option<String>,
        /// Skip the password prompt and use this PAT directly
        #[arg(long)]
        token: Option<String>,
        /// Username (skips the interactive prompt; use with --password)
        #[arg(long, short = 'u')]
        username: Option<String>,
        /// Password (skips the interactive prompt; avoid in shared shells)
        #[arg(long, short = 'p')]
        password: Option<String>,
        /// Automatically trust the server's TLS certificate on first
        /// connect without prompting. Use in CI / scripts only — interactive
        /// users should verify the fingerprint manually.
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Forget the stored credential for a server (and revoke its session)
    Logout {
        /// Server URL (defaults to current workspace's remote)
        #[arg(long)]
        server: Option<String>,
    },

    /// Show the authenticated user for a forge server
    Whoami {
        /// Server URL (defaults to current workspace's remote)
        #[arg(long)]
        server: Option<String>,
    },

    /// Inspect a raw object (snapshot, tree, blob)
    #[command(name = "cat-object")]
    CatObject {
        /// Object hash (full or short) or ref name
        object: String,
    },

    /// Print client version; inside a repo, also print the server version.
    #[command(visible_alias = "info")]
    Version,

    /// Pin a forge server's self-signed TLS certificate (trust on first use).
    ///
    /// Connects to the given `https://<host>:<port>` URL, captures the
    /// presented certificate chain, prints the CA fingerprint for manual
    /// comparison, and — on confirmation — saves the trust anchor to
    /// `~/.forge/trusted/<host>_<port>.pem`. Subsequent forge CLI calls to
    /// that server will use the pinned certificate automatically.
    Trust {
        /// Full server URL, e.g. `https://forge.example.com:9876`
        server: String,
        /// Skip the interactive confirmation prompt (scripts, CI).
        #[arg(long)]
        yes: bool,
    },

    /// Manage repository secrets (CI tokens, signing keys, …). Values can
    /// be written or replaced but never read back — the server exposes
    /// them to workflow runs only, with automatic log masking.
    Secrets {
        #[command(subcommand)]
        action: SecretsAction,
    },

    /// Manage CI workflows.
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },

    /// Inspect workflow runs.
    Runs {
        #[command(subcommand)]
        action: RunsAction,
    },

    /// Manage build artifacts.
    Artifacts {
        #[command(subcommand)]
        action: ArtifactsAction,
    },
}

#[derive(Subcommand)]
enum SecretsAction {
    /// Store a secret (interactive prompt if neither --value nor --file).
    Set {
        key: String,
        #[arg(long)]
        value: Option<String>,
        #[arg(long)]
        file: Option<String>,
    },
    /// Delete a secret.
    Delete { key: String },
    /// List secret keys (values are never returned).
    List,
}

#[derive(Subcommand)]
enum WorkflowAction {
    /// List workflows configured for this repo.
    List,
    /// Create a new workflow from a YAML file.
    Create { name: String, file: String },
    /// Delete a workflow by id.
    Delete { id: i64 },
    /// Enable a disabled workflow.
    Enable { id: i64 },
    /// Disable a workflow without deleting it.
    Disable { id: i64 },
    /// Manually trigger a run for a workflow.
    Trigger {
        workflow_id: i64,
        /// Branch / ref to record against the run. Default: empty.
        #[arg(long, value_name = "REF")]
        r#ref: Option<String>,
    },
}

#[derive(Subcommand)]
enum RunsAction {
    /// List runs in the current repo.
    List {
        #[arg(long)]
        workflow: Option<i64>,
        #[arg(long, default_value_t = 50)]
        limit: i32,
    },
    /// Show run detail + steps + artifacts.
    Show { run_id: i64 },
    /// Tail step logs (catch-up + live follow). Pass --step to filter.
    Logs {
        run_id: i64,
        #[arg(long, default_value_t = 0)]
        step: i64,
        /// Stop after replaying persisted log; don't follow.
        #[arg(long)]
        no_follow: bool,
    },
    /// Cancel a running or queued run.
    Cancel { run_id: i64 },
}

#[derive(Subcommand)]
enum ArtifactsAction {
    /// List artifacts for a run.
    List { run_id: i64 },
    /// Stream an artifact to local disk.
    Download {
        artifact_id: i64,
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
}

fn main() {
    tracing_subscriber::fmt::init();

    // Install a rustls crypto provider so `https://` server URLs work.
    // See twin call in forge-server/forge-web main — multiple provider crates
    // can end up in the build, so we pick aws-lc-rs explicitly.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let cli = Cli::parse();
    if let Err(err) = run_cli(cli) {
        print_pretty_error(err);
        std::process::exit(1);
    }
}

fn run_cli(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Init => commands::init::run()?,
        Commands::Add { paths } => commands::add::run(paths)?,
        Commands::Commit { message, all, amend } => commands::snapshot::run(message, all, amend, cli.json)?,
        Commands::Status => commands::status::run(cli.json)?,
        Commands::Diff { commit, staged, stat, extract, no_pager, class_stats, paths } => commands::diff::run(commit, staged, stat, extract, paths, no_pager, cli.json, class_stats)?,
        Commands::Log { count, file, oneline, all, no_pager } => commands::log::run(count, file, oneline, all, no_pager, cli.json)?,
        Commands::Push { force, remote, branch } => {
            commands::push::run(force, remote.as_deref(), branch.as_deref())?
        }
        Commands::Pull => commands::pull::run()?,
        Commands::Fetch { branch } => commands::fetch::run(branch)?,
        Commands::Clone { url, repo, path } => commands::clone::run(url, path, repo)?,
        Commands::Lock { path, reason } => commands::lock::run(path, reason, cli.json)?,
        Commands::Unlock { path, force } => commands::unlock::run(path, force, cli.json)?,
        Commands::Locks => commands::locks::run(cli.json)?,
        Commands::Unstage { paths } => commands::unstage::run(paths)?,
        Commands::Restore { staged, source, paths } => commands::restore::run(staged, source, paths)?,
        Commands::Branch { name, delete, all, remotes } => commands::branch::run(name, delete, all, remotes, cli.json)?,
        Commands::Switch { name, create } => commands::switch::run_with_create(name, create)?,
        Commands::Ignore { patterns } => commands::ignore::run(patterns)?,
        Commands::Remote { action, args } => commands::remote::run(action, args)?,
        Commands::Config { key, value } => commands::config_cmd::run(key, value)?,
        Commands::Tag { name, commit, delete } => commands::tag::run(name, commit, delete)?,
        Commands::Rm { paths, cached, recursive } => commands::rm::run(paths, cached, recursive)?,
        Commands::Mv { source, dest } => commands::mv::run(source, dest)?,
        Commands::Show { commit } => commands::show::run(commit, cli.json)?,
        Commands::Merge { branch } => commands::merge::run(branch)?,
        Commands::CherryPick { commit } => commands::cherry_pick::run(commit)?,
        Commands::Checkout { target, paths } => commands::checkout::run(target, paths)?,
        Commands::Clean { force, directories } => commands::clean::run(force, directories)?,
        Commands::Reset { commit, soft, hard } => commands::reset::run(commit, soft, hard)?,
        Commands::Stash { action, message } => commands::stash::run(action, message)?,
        Commands::Revert { commit } => commands::revert::run(commit)?,
        Commands::AssetInfo { path } => commands::asset_info::run(path, cli.json)?,
        Commands::Gc { dry_run } => commands::gc::run(dry_run)?,
        Commands::Update { check, force } => commands::update::run(check, force, cli.json)?,
        Commands::Login { server, token, username, password, yes } => commands::login::run(server, token, username, password, yes)?,
        Commands::Logout { server } => commands::logout::run(server)?,
        Commands::Whoami { server } => commands::whoami::run(server)?,
        Commands::CatObject { object } => commands::cat_object::run(object)?,
        Commands::Version => commands::version::run(cli.json)?,
        Commands::Trust { server, yes } => commands::trust::run(server, yes)?,
        Commands::Secrets { action } => match action {
            SecretsAction::Set { key, value, file } => {
                commands::secrets::set(&key, value, file, cli.json)?
            }
            SecretsAction::Delete { key } => commands::secrets::delete(&key, cli.json)?,
            SecretsAction::List => commands::secrets::list(cli.json)?,
        },
        Commands::Workflow { action } => match action {
            WorkflowAction::List => commands::workflow::list(cli.json)?,
            WorkflowAction::Create { name, file } => {
                commands::workflow::create(&name, &file, cli.json)?
            }
            WorkflowAction::Delete { id } => commands::workflow::delete(id, cli.json)?,
            WorkflowAction::Enable { id } => {
                commands::workflow::set_enabled(id, true, cli.json)?
            }
            WorkflowAction::Disable { id } => {
                commands::workflow::set_enabled(id, false, cli.json)?
            }
            WorkflowAction::Trigger { workflow_id, r#ref } => {
                commands::workflow::trigger(workflow_id, r#ref, cli.json)?
            }
        },
        Commands::Runs { action } => match action {
            RunsAction::List { workflow, limit } => {
                commands::runs::list(workflow.unwrap_or(0), limit, cli.json)?
            }
            RunsAction::Show { run_id } => commands::runs::show(run_id, cli.json)?,
            RunsAction::Logs { run_id, step, no_follow } => {
                commands::runs::logs(run_id, step, !no_follow, cli.json)?
            }
            RunsAction::Cancel { run_id } => commands::runs::cancel(run_id, cli.json)?,
        },
        Commands::Artifacts { action } => match action {
            ArtifactsAction::List { run_id } => commands::artifacts::list(run_id, cli.json)?,
            ArtifactsAction::Download { artifact_id, out } => {
                commands::artifacts::download(artifact_id, out)?
            }
        },
    }

    Ok(())
}

// ── Pretty error reporting ───────────────────────────────────────────────────

/// Pretty-print an error from any subcommand. The interesting case is a
/// `tonic::Status` bubbled up from a gRPC call — those carry rich metadata
/// that's noisy as a Display impl, so we strip it down to a one-line message
/// and offer a contextual next step (login prompt for Unauthenticated, etc).
fn print_pretty_error(err: anyhow::Error) {
    if let Some(status) = find_tonic_status(&err) {
        // Edge replica refused a write and handed us the primary's URL.
        // Persist the mapping so the next run's `connect_forge_write`
        // silently redirects, then print a concrete retry suggestion so
        // the current run can proceed without the user guessing the
        // flag to pass.
        if let Some(primary) = forge_client::edge::extract_upstream_hint(status) {
            if let Some(edge_url) = server_url_hint() {
                forge_client::edge::record_edge_upstream(&edge_url, &primary);
            } else if let Ok(cwd) = std::env::current_dir() {
                if let Ok(ws) = forge_core::workspace::Workspace::discover(&cwd) {
                    if let Ok(cfg) = ws.config() {
                        if let Some(edge_url) = cfg.default_remote_url() {
                            forge_client::edge::record_edge_upstream(edge_url, &primary);
                        }
                    }
                }
            }
            eprintln!("\x1b[1;31merror:\x1b[0m read-only edge replica refused this write");
            eprintln!("       \x1b[2m{}\x1b[0m", status.message());
            eprintln!();
            eprintln!("The primary server is at \x1b[1m{primary}\x1b[0m.");
            eprintln!(
                "This mapping has been cached; the next write against the same \
                 edge URL will be routed to the primary automatically."
            );
            eprintln!("Re-run the command now to apply the redirect.");
            return;
        }
        match status.code() {
            tonic::Code::Unauthenticated => {
                eprintln!("\x1b[1;31merror:\x1b[0m not authenticated");
                if !status.message().is_empty() {
                    eprintln!("       \x1b[2m{}\x1b[0m", status.message());
                }
                eprintln!();
                offer_login();
                return;
            }
            tonic::Code::PermissionDenied => {
                eprintln!("\x1b[1;31merror:\x1b[0m permission denied");
                if !status.message().is_empty() {
                    eprintln!("       \x1b[2m{}\x1b[0m", status.message());
                }
                eprintln!();
                eprintln!(
                    "Your account is logged in but doesn't have the required role on this resource."
                );
                eprintln!("Ask a server admin to grant access.");
                return;
            }
            tonic::Code::NotFound => {
                eprintln!("\x1b[1;31merror:\x1b[0m {}", status.message());
                return;
            }
            tonic::Code::FailedPrecondition | tonic::Code::AlreadyExists => {
                eprintln!("\x1b[1;31merror:\x1b[0m {}", status.message());
                return;
            }
            tonic::Code::Unavailable => {
                eprintln!("\x1b[1;31merror:\x1b[0m forge server unavailable");
                if !status.message().is_empty() {
                    eprintln!("       \x1b[2m{}\x1b[0m", status.message());
                }
                eprintln!();
                eprintln!("Check that the server is running and reachable.");
                return;
            }
            _ => {
                eprintln!("\x1b[1;31merror:\x1b[0m {}", status.message());
                return;
            }
        }
    }
    eprintln!("\x1b[1;31merror:\x1b[0m {err}");
}

/// Walk an `anyhow::Error` chain looking for a `tonic::Status`.
fn find_tonic_status(err: &anyhow::Error) -> Option<&tonic::Status> {
    if let Some(s) = err.downcast_ref::<tonic::Status>() {
        return Some(s);
    }
    let mut source: Option<&dyn std::error::Error> = err.source();
    while let Some(s) = source {
        if let Some(status) = s.downcast_ref::<tonic::Status>() {
            return Some(status);
        }
        source = s.source();
    }
    None
}

/// Print a "Run forge login" suggestion and, if the user is on a TTY,
/// prompt to run it inline. After the login completes, the user re-runs
/// their original command.
fn offer_login() {
    use std::io::{IsTerminal, Write};

    // Resolution order for the --server URL we'll pre-fill:
    //   1. Hint stashed by the running command (e.g. `forge clone <url>`
    //      sets this before the workspace even exists).
    //   2. Default remote from a discovered workspace under cwd.
    let server = server_url_hint()
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|cwd| forge_core::workspace::Workspace::discover(&cwd).ok())
                .and_then(|ws| ws.config().ok())
                .and_then(|c| c.default_remote_url().map(str::to_string))
        });

    let suggestion = match &server {
        Some(s) => format!("forge login --server {s}"),
        None => "forge login --server <url>".to_string(),
    };

    eprintln!("To authenticate, run:");
    eprintln!();
    eprintln!("    {suggestion}");
    eprintln!();

    // Only prompt interactively if both stdin and stderr are TTYs — never
    // try to prompt from CI / piped input.
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return;
    }

    eprint!("Login now? [Y/n] ");
    let _ = std::io::stderr().flush();
    let mut buf = String::new();
    if std::io::stdin().read_line(&mut buf).is_err() {
        return;
    }
    let answer = buf.trim().to_lowercase();
    if !(answer.is_empty() || answer == "y" || answer == "yes") {
        return;
    }

    // Run the regular interactive login flow. It'll prompt for username +
    // password (rpassword), call AuthService::Login, mint a PAT, and store
    // it in the keychain / credentials file.
    if let Err(e) = commands::login::run(server, None, None, None, false) {
        eprintln!("\x1b[1;31mlogin failed:\x1b[0m {e}");
        return;
    }
    eprintln!();
    eprintln!("\x1b[32mLogged in.\x1b[0m Re-run your command.");
}
