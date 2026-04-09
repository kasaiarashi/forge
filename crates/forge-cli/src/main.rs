mod client;
mod commands;
mod credentials;
mod pager;
mod tofu;
mod url_resolver;

use clap::{Parser, Subcommand};
use std::cell::RefCell;

// Thread-local "current command server URL hint". Commands that know the
// target server up-front (`forge clone <url>`) stash it here so that if
// auth fails mid-execution, `offer_login` prompts with the correct URL
// instead of asking the user to re-type it.
thread_local! {
    static SERVER_URL_HINT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Set the current command's server URL hint. Called by commands that
/// carry an explicit URL argument (e.g. `clone`).
pub(crate) fn set_server_url_hint(url: impl Into<String>) {
    SERVER_URL_HINT.with(|h| *h.borrow_mut() = Some(url.into()));
}

/// Read the hint. `None` when no command stashed one.
fn server_url_hint() -> Option<String> {
    SERVER_URL_HINT.with(|h| h.borrow().clone())
}

#[derive(Parser)]
#[command(name = "forge", about = "Version control for Unreal Engine", version)]
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

        /// File paths to restrict diff to
        paths: Vec<String>,
    },

    /// Show commit history
    Log {
        /// Number of commits to show
        #[arg(short, long, default_value_t = 20)]
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
    },

    /// Pull commits from the server
    Pull,

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

    /// Print client version; inside a repo, also print the server version.
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
        Commands::Diff { commit, staged, stat, extract, no_pager, paths } => commands::diff::run(commit, staged, stat, extract, paths, no_pager, cli.json)?,
        Commands::Log { count, file, oneline, all, no_pager } => commands::log::run(count, file, oneline, all, no_pager, cli.json)?,
        Commands::Push { force } => commands::push::run(force)?,
        Commands::Pull => commands::pull::run()?,
        Commands::Clone { url, repo, path } => commands::clone::run(url, path, repo)?,
        Commands::Lock { path, reason } => commands::lock::run(path, reason, cli.json)?,
        Commands::Unlock { path, force } => commands::unlock::run(path, force, cli.json)?,
        Commands::Locks => commands::locks::run(cli.json)?,
        Commands::Unstage { paths } => commands::unstage::run(paths)?,
        Commands::Restore { staged, source, paths } => commands::restore::run(staged, source, paths)?,
        Commands::Branch { name, delete } => commands::branch::run(name, delete, cli.json)?,
        Commands::Switch { name, create } => commands::switch::run_with_create(name, create)?,
        Commands::Ignore { patterns } => commands::ignore::run(patterns)?,
        Commands::Remote { action, args } => commands::remote::run(action, args)?,
        Commands::Config { key, value } => commands::config_cmd::run(key, value)?,
        Commands::Tag { name, commit, delete } => commands::tag::run(name, commit, delete)?,
        Commands::Rm { paths, cached } => commands::rm::run(paths, cached)?,
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
        Commands::Update { check } => commands::update::run(check, cli.json)?,
        Commands::Login { server, token, username, password, yes } => commands::login::run(server, token, username, password, yes)?,
        Commands::Logout { server } => commands::logout::run(server)?,
        Commands::Whoami { server } => commands::whoami::run(server)?,
        Commands::Version => commands::version::run(cli.json)?,
        Commands::Trust { server, yes } => commands::trust::run(server, yes)?,
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
