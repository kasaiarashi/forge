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

    /// Create a snapshot of current changes
    Snapshot {
        /// Snapshot message
        #[arg(short, long)]
        message: String,

        /// Snapshot all changed files (skip explicit staging)
        #[arg(short, long)]
        all: bool,
    },

    /// Show working directory status
    Status,

    /// Show differences
    Diff {
        /// Compare against a specific snapshot
        #[arg(long)]
        snapshot: Option<String>,
    },

    /// Show snapshot history
    Log {
        /// Number of snapshots to show
        #[arg(short, long, default_value_t = 20)]
        count: u32,

        /// Show history for a specific file
        #[arg(long)]
        file: Option<String>,
    },

    /// Push snapshots to the server
    Push,

    /// Pull snapshots from the server
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
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init => commands::init::run()?,
        Commands::Add { paths } => commands::add::run(paths)?,
        Commands::Snapshot { message, all } => commands::snapshot::run(message, all)?,
        Commands::Status => commands::status::run(cli.json)?,
        Commands::Diff { snapshot } => commands::diff::run(snapshot)?,
        Commands::Log { count, file } => commands::log::run(count, file)?,
        Commands::Push => commands::push::run()?,
        Commands::Pull => commands::pull::run()?,
        Commands::Clone { url, path } => commands::clone::run(url, path)?,
        Commands::Lock { path, reason } => commands::lock::run(path, reason)?,
        Commands::Unlock { path, force } => commands::unlock::run(path, force)?,
        Commands::Locks => commands::locks::run()?,
        Commands::Branch { name, delete } => commands::branch::run(name, delete)?,
        Commands::Switch { name } => commands::switch::run(name)?,
        Commands::Ignore { patterns } => commands::ignore::run(patterns)?,
    }

    Ok(())
}
