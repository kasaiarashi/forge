//! Asset-aware diff engine for Forge VCS.
//!
//! This crate owns everything that turns two byte blobs into a structured,
//! human-readable diff: UE asset parsing glue, per-export-type handlers, the
//! K2Node pin parser, and the output formatters (colored / JSON / stat / extract).
//!
//! The generic tree-diff primitives (`flatten_tree`, `diff_maps`, `DiffEntry`)
//! live in `forge_core::diff` and are used by many commands; they are not moved
//! here.
//!
//! # Extending with a new asset type
//!
//! Implement [`handler::AssetDiffHandler`] and register your instance on a
//! [`handler::HandlerRegistry`]. The default registry is built by
//! [`handler::HandlerRegistry::default`] and wires in all built-in handlers
//! (imports, tagged properties, Blueprint variables, enum values, field
//! definitions, K2Node pins).

pub mod asset_paths;
pub mod change;
pub mod engine;
pub mod format;
pub mod handler;
pub mod k2node;
pub mod label;

// Public surface mirrors the legacy `forge_core::uasset_diff` module so callers
// only need to update their `use` path.
pub use change::AssetChange;
pub use engine::{diff_assets, diff_assets_with_data};
pub use label::extract_k2node_label;

// Re-exports for use by forge-cli without depending on uasset directly.
pub use forge_unreal::ffield;
pub use forge_unreal::structured::parse_structured;
pub use forge_unreal::structured::parse_structured_with_uexp;
pub use forge_unreal::structured::{ExportInfo, ImportInfo, StructuredAsset};
