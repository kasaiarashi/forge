use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A server-side file lock record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lock {
    /// Repository-relative path (forward-slash normalized).
    pub path: String,
    /// Who holds the lock.
    pub owner: String,
    /// Which workspace instance.
    pub workspace_id: String,
    /// When the lock was acquired.
    pub created_at: DateTime<Utc>,
    /// Optional reason or note.
    pub reason: Option<String>,
}
