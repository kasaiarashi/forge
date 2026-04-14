//! Export identity helpers — match keys and human-friendly labels.

use forge_unreal::property::PropertyValue;
use forge_unreal::structured::ExportInfo;

/// Build the match key used to pair exports between old and new.
///
/// UE recycles auto-numbered object names like `K2Node_CallFunction_42` across
/// unrelated logical nodes on re-save: after adding a new graph node, an
/// existing node can end up holding a different `_N` suffix, and the newly
/// added node may claim a suffix a pre-existing node once used. Pairing by
/// `object_name` therefore silently aliases different logical nodes.
///
/// UEdGraphNode carries a stable `NodeGuid` UPROPERTY (set at node-creation
/// time, never rewritten). For any K2Node export that has a parseable
/// `NodeGuid` tagged property, we pair on that GUID instead. For every other
/// export class, the `object_name` remains the pairing key — those don't
/// suffer from the renumber-aliasing problem.
pub(crate) fn match_key(exp: &ExportInfo) -> String {
    if exp.class_name.starts_with("K2Node_") {
        if let Some(guid) = extract_node_guid(exp) {
            return format!("guid:{}", guid);
        }
    }
    // Fall back to (outer, object_name) — object_name alone can alias across
    // different outers (e.g. same K2Node name nested under two EventGraphs).
    match &exp.outer_name {
        Some(outer) => format!("name:{}::{}", outer, exp.object_name),
        None => format!("name:{}", exp.object_name),
    }
}

/// Extract a human-friendly label for a K2Node export, if one can be recovered
/// from its raw data.
///
/// Why this is a byte-scan and not a tagged-property lookup: the uasset tagged
/// property parser yields `Some(vec![])` for every K2Node export in practice
/// (778/778 on a real UE 5.7 Blueprint). So `exp.properties.find("MemberName")`
/// never finds anything. Instead we scan the export's serialized byte range
/// for the FName-tag pattern that precedes a `MemberName` NameProperty — the
/// same approach that already works for `scan_blueprint_variables`.
///
/// Recognised node classes and what we surface:
/// - `K2Node_CallFunction`  → function name (e.g. `PrintString`)
/// - `K2Node_VariableGet` / `K2Node_VariableSet` → variable name
/// - `K2Node_Event` / `K2Node_CustomEvent`      → event name
/// - `K2Node_MacroInstance`                     → macro name
///
/// Returns `None` when the export isn't a labellable K2Node class, the pattern
/// isn't found, or the required name-table entries are absent. Callers fall
/// back to the class name for display.
pub fn extract_k2node_label(
    exp: &ExportInfo,
    file_data: Option<&[u8]>,
    names: &[String],
) -> Option<String> {
    if !exp.class_name.starts_with("K2Node_") {
        return None;
    }
    let data = file_data?;
    let start = exp.serial_offset as usize;
    let end = start.checked_add(exp.serial_size as usize)?;
    if end > data.len() {
        return None;
    }
    let slice = &data[start..end];

    let member_idx = names.iter().position(|n| n == "MemberName")?;
    let name_prop_idx = names.iter().position(|n| n == "NameProperty")?;

    let mn_bytes = (member_idx as u32).to_le_bytes();
    let np_bytes = (name_prop_idx as u32).to_le_bytes();
    let zero = 0u32.to_le_bytes();

    let min_len = 25 + 8;
    if slice.len() < min_len {
        return None;
    }
    for off in 0..=slice.len() - min_len {
        if slice[off..off + 4] != mn_bytes { continue; }
        if slice[off + 4..off + 8] != zero { continue; }
        if slice[off + 8..off + 12] != np_bytes { continue; }
        if slice[off + 12..off + 16] != zero { continue; }
        let val_off = off + 25;
        let name_idx = u32::from_le_bytes(slice[val_off..val_off + 4].try_into().ok()?) as usize;
        let name_num = u32::from_le_bytes(slice[val_off + 4..val_off + 8].try_into().ok()?);
        if name_idx >= names.len() { continue; }
        let base = &names[name_idx];
        let label = if name_num > 0 {
            format!("{}_{}", base, name_num - 1)
        } else {
            base.clone()
        };
        return Some(label);
    }
    None
}

/// Extract a `NodeGuid` hex string from a K2Node export's tagged properties.
/// Returns `None` if the property isn't present or parsing didn't reach it.
fn extract_node_guid(exp: &ExportInfo) -> Option<String> {
    let props = exp.properties.as_ref()?;
    let ng = props.iter().find(|p| p.name == "NodeGuid")?;
    if let PropertyValue::Struct { fields, .. } = &ng.value {
        let value_field = fields.iter().find(|f| f.name == "Value")?;
        if let PropertyValue::Str(hex) = &value_field.value {
            return Some(hex.clone());
        }
    }
    None
}
