mod commands;

use clap::{Parser, Subcommand};

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
        /// Commit message
        #[arg(short, long)]
        message: String,

        /// Commit all changed files (skip explicit staging)
        #[arg(short, long)]
        all: bool,
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

    /// Switch to a branch
    Switch {
        /// Branch name
        name: String,
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
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init => commands::init::run()?,
        Commands::Add { paths } => commands::add::run(paths)?,
        Commands::Commit { message, all } => commands::snapshot::run(message, all, cli.json)?,
        Commands::Status => commands::status::run(cli.json)?,
        Commands::Diff { commit, staged, stat, extract, paths } => commands::diff::run(commit, staged, stat, extract, paths, cli.json)?,
        Commands::Log { count, file, oneline, all } => commands::log::run(count, file, oneline, all, cli.json)?,
        Commands::Push { force } => commands::push::run(force)?,
        Commands::Pull => commands::pull::run()?,
        Commands::Clone { url, path } => commands::clone::run(url, path)?,
        Commands::Lock { path, reason } => commands::lock::run(path, reason, cli.json)?,
        Commands::Unlock { path, force } => commands::unlock::run(path, force, cli.json)?,
        Commands::Locks => commands::locks::run(cli.json)?,
        Commands::Unstage { paths } => commands::unstage::run(paths)?,
        Commands::Restore { staged, source, paths } => commands::restore::run(staged, source, paths)?,
        Commands::Branch { name, delete } => commands::branch::run(name, delete, cli.json)?,
        Commands::Switch { name } => commands::switch::run(name)?,
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
    }

    Ok(())
}
