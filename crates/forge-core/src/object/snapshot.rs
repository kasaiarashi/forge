use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::hash::ForgeHash;

/// An immutable point-in-time capture of the project state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Hash of the root Tree object.
    pub tree: ForgeHash,
    /// Zero parents = initial, one = normal, two = merge.
    pub parents: Vec<ForgeHash>,
    /// Who created this snapshot.
    pub author: Author,
    /// Descriptive message.
    pub message: String,
    /// When this snapshot was created.
    pub timestamp: DateTime<Utc>,
    /// Optional metadata (e.g., UE project version).
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub email: String,
}
