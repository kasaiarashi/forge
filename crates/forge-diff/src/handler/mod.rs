//! Plugin-style asset diff handlers.
//!
//! The engine calls into a [`HandlerRegistry`]. Built-in handlers cover
//! imports, tagged properties, field definitions, Blueprint variables,
//! UserDefinedEnum values, and (feature-gated) K2Node pins. New asset types
//! plug in by implementing [`AssetDiffHandler`] and calling
//! [`HandlerRegistry::register`].
//!
//! Split into separate sub-modules per concern so each handler is an
//! independently reviewable unit.

pub mod blueprint_vars;
pub mod enum_values;
pub mod field_defs;
pub mod imports;
pub mod k2node_pins;
pub mod properties;

use crate::change::AssetChange;
use forge_unreal::structured::{ExportInfo, StructuredAsset};

/// Bundles one side of a diff (either the "old" or "new" asset) so handlers
/// don't need to juggle four parallel arguments.
#[derive(Clone, Copy)]
pub struct AssetSide<'a> {
    pub asset: &'a StructuredAsset,
    pub raw_data: Option<&'a [u8]>,
}

impl<'a> AssetSide<'a> {
    pub fn new(asset: &'a StructuredAsset, raw_data: Option<&'a [u8]>) -> Self {
        Self { asset, raw_data }
    }

    pub fn names(&self) -> &'a [String] {
        &self.asset.names
    }
}

/// Context passed to every handler call — both sides of the diff.
pub struct DiffContext<'a> {
    pub old: AssetSide<'a>,
    pub new: AssetSide<'a>,
}

/// Plug-in point for asset-type-specific diffing.
///
/// `diff_top_level` runs once per diff and is the entry point for handlers
/// that scan the asset as a whole (imports, Blueprint variables, enum values).
///
/// `diff_matched_export` runs once per (old, new) export pair already matched
/// by the engine via [`crate::label::match_key`]. It is the entry point for
/// handlers that operate on a single export's contents (tagged properties,
/// field definitions, K2Node pins).
pub trait AssetDiffHandler: Send + Sync {
    fn name(&self) -> &'static str;

    /// Called once per diff. Default is no-op.
    fn diff_top_level(&self, _ctx: &DiffContext<'_>, _sink: &mut Vec<AssetChange>) {}

    /// Called once per matched (old, new) export pair. Default is no-op.
    ///
    /// Returning `true` tells the engine that this handler consumed the pair —
    /// used by the K2Node pin handler to suppress the fallback
    /// "native data changed (X -> Y bytes)" message when it successfully
    /// decoded the trailing data.
    fn diff_matched_export(
        &self,
        _ctx: &DiffContext<'_>,
        _old_exp: &ExportInfo,
        _new_exp: &ExportInfo,
        _sink: &mut Vec<AssetChange>,
    ) -> bool {
        false
    }
}

/// Ordered list of handlers the engine runs.
///
/// Order matters: it determines the order of `AssetChange` entries in output
/// and therefore the formatter's display order. [`HandlerRegistry::default`]
/// matches the legacy `uasset_diff::diff_assets_with_data` emission order
/// exactly.
pub struct HandlerRegistry {
    handlers: Vec<Box<dyn AssetDiffHandler>>,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self { handlers: Vec::new() }
    }

    pub fn register(&mut self, h: Box<dyn AssetDiffHandler>) -> &mut Self {
        self.handlers.push(h);
        self
    }

    pub fn handlers(&self) -> &[Box<dyn AssetDiffHandler>] {
        &self.handlers
    }
}

impl Default for HandlerRegistry {
    fn default() -> Self {
        let mut r = Self::new();
        r.register(Box::new(imports::ImportHandler));
        // Per-export handlers (invoked by engine during export pair iteration):
        // PropertyHandler must run first so field/pin changes come after
        // property changes in the legacy output order.
        r.register(Box::new(properties::PropertyHandler));
        #[cfg(feature = "k2-diff")]
        r.register(Box::new(k2node_pins::K2NodePinHandler));
        #[cfg(not(feature = "k2-diff"))]
        r.register(Box::new(k2node_pins::K2NodePinHandler));
        r.register(Box::new(field_defs::FieldDefHandler));
        // Top-level scans, emitted after per-export changes to match legacy order.
        r.register(Box::new(blueprint_vars::BlueprintVarHandler));
        r.register(Box::new(enum_values::EnumHandler));
        r
    }
}
