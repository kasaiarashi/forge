pub mod snapshot;
pub mod tree;
pub mod blob;
pub mod lock;

use serde::{Deserialize, Serialize};

use crate::hash::ForgeHash;

/// Tag byte prefix for object type identification in storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ObjectType {
    Blob = 1,
    ChunkedBlob = 2,
    Tree = 3,
    Snapshot = 4,
}

/// An object ID is the ForgeHash of its serialized content.
pub type ObjectId = ForgeHash;
