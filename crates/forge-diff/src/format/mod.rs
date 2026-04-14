//! Output formatters — one per CLI flag (default colored, `--json`, `--stat`,
//! `--extract`). Each formatter consumes a list of [`FileDiff`] records and
//! writes a string (or in the case of `extract`, temp files).
//!
//! The structured-asset formatter lives in [`unified`]: it turns an
//! [`AssetChange`](crate::change::AssetChange) list into a hierarchical tree
//! view with K2Node label decoration and UE auto-renumber collapsing.

pub mod class_stats;
pub mod colored;
pub mod extract;
pub mod file_diff;
pub mod json;
pub mod renumber;
pub mod stat;
pub mod unified;

pub use file_diff::FileDiff;
