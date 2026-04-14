//! Top-level asset diff orchestrator.
//!
//! Iterates a [`HandlerRegistry`] in two phases: top-level scans (imports,
//! Blueprint variables, enum values) and per-matched-export-pair dispatch.
//! Export add/remove detection lives here, because matching uses the crate's
//! own [`match_key`](crate::label::match_key) logic.

use std::collections::BTreeMap;
use forge_unreal::structured::{ExportInfo, StructuredAsset};

use crate::change::AssetChange;
use crate::handler::{AssetSide, DiffContext, HandlerRegistry};
use crate::label::match_key;

/// Compare two structured assets and return a list of semantic changes.
pub fn diff_assets(old: &StructuredAsset, new: &StructuredAsset) -> Vec<AssetChange> {
    diff_assets_with_data(old, None, new, None)
}

/// Compare two structured assets with optional raw file data for deep scanning.
///
/// When raw data is provided, Blueprint exports are scanned for `NewVariables`
/// to detect added/removed Blueprint variables, and K2Node trailing native
/// data is decoded for pin-level changes.
pub fn diff_assets_with_data(
    old: &StructuredAsset,
    old_data: Option<&[u8]>,
    new: &StructuredAsset,
    new_data: Option<&[u8]>,
) -> Vec<AssetChange> {
    diff_assets_with_registry(old, old_data, new, new_data, &HandlerRegistry::default())
}

/// Run the diff against a caller-supplied registry (for custom handler sets).
pub fn diff_assets_with_registry(
    old: &StructuredAsset,
    old_data: Option<&[u8]>,
    new: &StructuredAsset,
    new_data: Option<&[u8]>,
    registry: &HandlerRegistry,
) -> Vec<AssetChange> {
    let mut changes = Vec::new();
    let ctx = DiffContext {
        old: AssetSide::new(old, old_data),
        new: AssetSide::new(new, new_data),
    };

    // Phase A: top-level scan for handlers that inspect imports (runs first
    // so ImportAdded/Removed land at the top of the output, matching legacy).
    for h in registry.handlers() {
        if h.name() == "imports" {
            h.diff_top_level(&ctx, &mut changes);
        }
    }

    // Phase B: per-export pass. Match exports, emit add/remove, then run every
    // per-export handler for each matched pair.
    diff_exports_with_handlers(&ctx, registry, &mut changes);

    // Phase C: remaining top-level scans (blueprint_vars, enum_values).
    for h in registry.handlers() {
        match h.name() {
            "imports" => continue,
            _ => h.diff_top_level(&ctx, &mut changes),
        }
    }

    changes
}

fn diff_exports_with_handlers(
    ctx: &DiffContext<'_>,
    registry: &HandlerRegistry,
    changes: &mut Vec<AssetChange>,
) {
    let old: &[ExportInfo] = &ctx.old.asset.exports;
    let new: &[ExportInfo] = &ctx.new.asset.exports;

    let old_map: BTreeMap<String, &ExportInfo> =
        old.iter().map(|e| (match_key(e), e)).collect();
    let new_map: BTreeMap<String, &ExportInfo> =
        new.iter().map(|e| (match_key(e), e)).collect();

    // Removed exports — use the export's own object_name for display, not the
    // match key (match key may be "guid:…" for K2Node exports).
    for (key, exp) in &old_map {
        if !new_map.contains_key(key) {
            changes.push(AssetChange::ExportRemoved {
                name: exp.object_name.clone(),
                class: exp.class_name.clone(),
            });
        }
    }

    // Added exports.
    for (key, exp) in &new_map {
        if !old_map.contains_key(key) {
            changes.push(AssetChange::ExportAdded {
                name: exp.object_name.clone(),
                class: exp.class_name.clone(),
            });
        }
    }

    // Modified exports — dispatch to each per-export handler.
    for (key, old_exp) in &old_map {
        if let Some(new_exp) = new_map.get(key) {
            for h in registry.handlers() {
                h.diff_matched_export(ctx, old_exp, new_exp, changes);
            }
        }
    }
}
