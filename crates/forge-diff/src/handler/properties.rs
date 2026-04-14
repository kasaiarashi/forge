//! Tagged-property diff — per-export property adds, removes, changes.
//!
//! Handles struct properties recursively so nested changes show as
//! `parent.child` paths in output.

use std::collections::BTreeMap;
use forge_unreal::property::{PropertyValue, TaggedProperty};
use forge_unreal::structured::ExportInfo;

use super::{AssetDiffHandler, DiffContext};
use crate::change::AssetChange;

pub struct PropertyHandler;

impl AssetDiffHandler for PropertyHandler {
    fn name(&self) -> &'static str { "properties" }

    fn diff_matched_export(
        &self,
        _ctx: &DiffContext<'_>,
        old_exp: &ExportInfo,
        new_exp: &ExportInfo,
        sink: &mut Vec<AssetChange>,
    ) -> bool {
        let display_name = new_exp.object_name.as_str();
        match (&old_exp.properties, &new_exp.properties) {
            (Some(old_props), Some(new_props)) => {
                diff_properties(display_name, old_props, new_props, sink);
            }
            (Some(_), None) | (None, Some(_)) => {
                sink.push(AssetChange::ExportDataChanged {
                    export_name: display_name.to_string(),
                    description: "property parsing changed between versions".to_string(),
                });
            }
            (None, None) => {
                if old_exp.serial_size != new_exp.serial_size {
                    sink.push(AssetChange::ExportDataChanged {
                        export_name: display_name.to_string(),
                        description: format!(
                            "binary data changed ({} -> {} bytes)",
                            old_exp.serial_size, new_exp.serial_size
                        ),
                    });
                }
            }
        }
        false
    }
}

fn diff_properties(
    export_name: &str,
    old_props: &[TaggedProperty],
    new_props: &[TaggedProperty],
    changes: &mut Vec<AssetChange>,
) {
    let old_map: BTreeMap<(&str, u32), &TaggedProperty> = old_props
        .iter()
        .map(|p| ((p.name.as_str(), p.array_index), p))
        .collect();
    let new_map: BTreeMap<(&str, u32), &TaggedProperty> = new_props
        .iter()
        .map(|p| ((p.name.as_str(), p.array_index), p))
        .collect();

    for ((name, idx), prop) in &old_map {
        if !new_map.contains_key(&(*name, *idx)) {
            let prop_name = if *idx > 0 {
                format!("{}[{}]", name, idx)
            } else {
                name.to_string()
            };
            changes.push(AssetChange::PropertyRemoved {
                export_name: export_name.to_string(),
                property_name: prop_name,
                value: format!("{}", prop.value),
            });
        }
    }

    for ((name, idx), prop) in &new_map {
        if !old_map.contains_key(&(*name, *idx)) {
            let prop_name = if *idx > 0 {
                format!("{}[{}]", name, idx)
            } else {
                name.to_string()
            };
            changes.push(AssetChange::PropertyAdded {
                export_name: export_name.to_string(),
                property_name: prop_name,
                value: format!("{}", prop.value),
            });
        }
    }

    for ((name, idx), old_prop) in &old_map {
        if let Some(new_prop) = new_map.get(&(*name, *idx)) {
            if old_prop.value != new_prop.value {
                let prop_path = if *idx > 0 {
                    format!("{}[{}]", name, idx)
                } else {
                    name.to_string()
                };

                if let (
                    PropertyValue::Struct { fields: old_fields, .. },
                    PropertyValue::Struct { fields: new_fields, .. },
                ) = (&old_prop.value, &new_prop.value)
                {
                    diff_struct_fields(export_name, &prop_path, old_fields, new_fields, changes);
                } else {
                    changes.push(AssetChange::PropertyChanged {
                        export_name: export_name.to_string(),
                        property_path: prop_path,
                        old_value: format!("{}", old_prop.value),
                        new_value: format!("{}", new_prop.value),
                    });
                }
            }
        }
    }
}

fn diff_struct_fields(
    export_name: &str,
    parent_path: &str,
    old_fields: &[TaggedProperty],
    new_fields: &[TaggedProperty],
    changes: &mut Vec<AssetChange>,
) {
    let old_map: BTreeMap<(&str, u32), &TaggedProperty> = old_fields
        .iter()
        .map(|p| ((p.name.as_str(), p.array_index), p))
        .collect();
    let new_map: BTreeMap<(&str, u32), &TaggedProperty> = new_fields
        .iter()
        .map(|p| ((p.name.as_str(), p.array_index), p))
        .collect();

    for ((name, idx), old_prop) in &old_map {
        let field_path = if *idx > 0 {
            format!("{}.{}[{}]", parent_path, name, idx)
        } else {
            format!("{}.{}", parent_path, name)
        };

        if let Some(new_prop) = new_map.get(&(*name, *idx)) {
            if old_prop.value != new_prop.value {
                changes.push(AssetChange::PropertyChanged {
                    export_name: export_name.to_string(),
                    property_path: field_path,
                    old_value: format!("{}", old_prop.value),
                    new_value: format!("{}", new_prop.value),
                });
            }
        } else {
            changes.push(AssetChange::PropertyRemoved {
                export_name: export_name.to_string(),
                property_name: field_path,
                value: format!("{}", old_prop.value),
            });
        }
    }

    for ((name, idx), new_prop) in &new_map {
        if !old_map.contains_key(&(*name, *idx)) {
            let field_path = if *idx > 0 {
                format!("{}.{}[{}]", parent_path, name, idx)
            } else {
                format!("{}.{}", parent_path, name)
            };
            changes.push(AssetChange::PropertyAdded {
                export_name: export_name.to_string(),
                property_name: field_path,
                value: format!("{}", new_prop.value),
            });
        }
    }
}
