//! K2Node pin-level diff — feature-gated Blueprint graph pin decode.
//!
//! When a K2Node export's trailing native data differs, try to decode pins on
//! both sides and emit fine-grained pin changes. If parsing fails or the class
//! isn't a K2Node, returns `false` so the engine falls back to the opaque
//! "native data changed (X -> Y bytes)" message.

use forge_unreal::structured::ExportInfo;

use super::{AssetDiffHandler, DiffContext};
use crate::change::AssetChange;

pub struct K2NodePinHandler;

impl AssetDiffHandler for K2NodePinHandler {
    fn name(&self) -> &'static str { "k2node_pins" }

    fn diff_matched_export(
        &self,
        ctx: &DiffContext<'_>,
        old_exp: &ExportInfo,
        new_exp: &ExportInfo,
        sink: &mut Vec<AssetChange>,
    ) -> bool {
        let has_field_changes = match (&old_exp.field_definitions, &new_exp.field_definitions) {
            (Some(old_fields), Some(new_fields)) => old_fields != new_fields,
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };

        if old_exp.trailing_data_size == new_exp.trailing_data_size || has_field_changes {
            return false;
        }

        let display_name = new_exp.object_name.as_str();

        let emitted = try_emit_k2node_pin_diff(
            display_name,
            old_exp, ctx.old.raw_data, ctx.old.names(),
            new_exp, ctx.new.raw_data, ctx.new.names(),
            sink,
        );
        if !emitted {
            sink.push(AssetChange::ExportDataChanged {
                export_name: display_name.to_string(),
                description: format!(
                    "native data changed ({} -> {} bytes)",
                    old_exp.trailing_data_size, new_exp.trailing_data_size
                ),
            });
        }
        // Regardless of whether pin-level diff or fallback message was emitted,
        // we have consumed the trailing-data diff for this export.
        true
    }
}

#[cfg(feature = "k2-diff")]
fn try_emit_k2node_pin_diff(
    export_name: &str,
    old_exp: &ExportInfo,
    old_data: Option<&[u8]>,
    old_names: &[String],
    new_exp: &ExportInfo,
    new_data: Option<&[u8]>,
    new_names: &[String],
    changes: &mut Vec<AssetChange>,
) -> bool {
    use crate::k2node::{self, K2NodeData};

    if !is_k2_node(&old_exp.class_name) || !is_k2_node(&new_exp.class_name) {
        return false;
    }
    let old_slice = slice_trailing(old_exp, old_data);
    let new_slice = slice_trailing(new_exp, new_data);
    let (Some(old_slice), Some(new_slice)) = (old_slice, new_slice) else { return false };

    let old_parsed = k2node::parse_k2_node(old_slice, old_names);
    let new_parsed = k2node::parse_k2_node(new_slice, new_names);
    let (K2NodeData::Parsed { pins: old_pins }, K2NodeData::Parsed { pins: new_pins })
        = (old_parsed, new_parsed) else { return false };

    use std::collections::HashMap;
    let old_by_id: HashMap<[u8; 16], &k2node::Pin> =
        old_pins.iter().map(|p| (p.pin_id, p)).collect();
    let new_by_id: HashMap<[u8; 16], &k2node::Pin> =
        new_pins.iter().map(|p| (p.pin_id, p)).collect();

    for (id, op) in &old_by_id {
        if !new_by_id.contains_key(id) {
            changes.push(AssetChange::PinRemoved {
                export_name: export_name.to_string(),
                pin_name: op.pin_name.clone(),
                pin_category: op.pin_category.clone(),
            });
        }
    }
    for (id, np) in &new_by_id {
        if !old_by_id.contains_key(id) {
            changes.push(AssetChange::PinAdded {
                export_name: export_name.to_string(),
                pin_name: np.pin_name.clone(),
                pin_category: np.pin_category.clone(),
                default_value: if np.default_value.is_empty() {
                    None
                } else {
                    Some(np.default_value.clone())
                },
            });
        }
    }
    for (id, op) in &old_by_id {
        let Some(np) = new_by_id.get(id) else { continue };
        if op.pin_name != np.pin_name {
            changes.push(AssetChange::PinRenamed {
                export_name: export_name.to_string(),
                old_name: op.pin_name.clone(),
                new_name: np.pin_name.clone(),
            });
        }
        if op.pin_category != np.pin_category {
            changes.push(AssetChange::PinTypeChanged {
                export_name: export_name.to_string(),
                pin_name: np.pin_name.clone(),
                old_category: op.pin_category.clone(),
                new_category: np.pin_category.clone(),
            });
        }
        if op.default_value != np.default_value {
            changes.push(AssetChange::PinDefaultChanged {
                export_name: export_name.to_string(),
                pin_name: np.pin_name.clone(),
                old_value: op.default_value.clone(),
                new_value: np.default_value.clone(),
            });
        }
        if op.linked_to.len() != np.linked_to.len() {
            changes.push(AssetChange::PinConnectionsChanged {
                export_name: export_name.to_string(),
                pin_name: np.pin_name.clone(),
                old_count: op.linked_to.len(),
                new_count: np.linked_to.len(),
            });
        }
    }

    true
}

#[cfg(not(feature = "k2-diff"))]
fn try_emit_k2node_pin_diff(
    _export_name: &str,
    _old_exp: &ExportInfo,
    _old_data: Option<&[u8]>,
    _old_names: &[String],
    _new_exp: &ExportInfo,
    _new_data: Option<&[u8]>,
    _new_names: &[String],
    _changes: &mut Vec<AssetChange>,
) -> bool {
    false
}

#[cfg(feature = "k2-diff")]
fn is_k2_node(class_name: &str) -> bool {
    class_name.starts_with("K2Node_")
}

#[cfg(feature = "k2-diff")]
fn slice_trailing<'a>(exp: &ExportInfo, data: Option<&'a [u8]>) -> Option<&'a [u8]> {
    let data = data?;
    if exp.trailing_data_size == 0 {
        return None;
    }
    let serial_offset = exp.serial_offset as usize;
    let serial_size = exp.serial_size as usize;
    let end = serial_offset.checked_add(serial_size)?;
    if end > data.len() {
        return None;
    }
    let start = end.checked_sub(exp.trailing_data_size)?;
    Some(&data[start..end])
}
