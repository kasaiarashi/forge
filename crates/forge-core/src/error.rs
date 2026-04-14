use thiserror::Error;

#[derive(Error, Debug)]
pub enum ForgeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Not a forge workspace (no .forge directory found)")]
    NotAWorkspace,

    #[error("Workspace already initialized at {0}")]
    AlreadyInitialized(String),

    #[error("Invalid hash: {0}")]
    InvalidHash(String),

    #[error("Object not found: {0}")]
    ObjectNotFound(String),

    #[error("File is locked by {owner} since {since}")]
    FileLocked {
        path: String,
        owner: String,
        since: String,
    },

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Merge conflict in: {0}")]
    Conflict(String),

    #[error("Dirty working tree: commit or discard changes first")]
    DirtyWorkingTree,

    #[error("Branch not found: {0}")]
    BranchNotFound(String),

    #[error("Branch already exists: {0}")]
    BranchAlreadyExists(String),

    #[error("{0}")]
    Other(String),
}
