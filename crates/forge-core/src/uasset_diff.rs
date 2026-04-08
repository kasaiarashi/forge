//! Structured diff engine for UE assets.
//!
//! Compares two `StructuredAsset` instances and produces a list of semantic changes
//! at the import, export, and property level.

use uasset::ffield::FieldDefinition;
use uasset::property::{PropertyValue, TaggedProperty};
use std::collections::BTreeMap;
use std::fmt;

// Re-export for use by forge-cli without depending on uasset directly.
pub use uasset::ffield;
pub use uasset::structured::parse_structured;
pub use uasset::structured::parse_structured_with_uexp;
pub use uasset::structured::{ExportInfo, ImportInfo, StructuredAsset};

/// A single semantic change within a UE asset.
#[derive(Debug)]
pub enum AssetChange {
    ImportAdded(ImportInfo),
    ImportRemoved(ImportInfo),
    ExportAdded {
        name: String,
        class: String,
    },
    ExportRemoved {
        name: String,
        class: String,
    },
    PropertyChanged {
        export_name: String,
        property_path: String,
        old_value: String,
        new_value: String,
    },
    PropertyAdded {
        export_name: String,
        property_name: String,
        value: String,
    },
    PropertyRemoved {
        export_name: String,
        property_name: String,
        value: String,
    },
    ExportDataChanged {
        export_name: String,
        description: String,
    },
    /// A variable/property definition was added to a class/struct.
    FieldAdded {
        export_name: String,
        field: FieldDefinition,
    },
    /// A variable/property definition was removed from a class/struct.
    FieldRemoved {
        export_name: String,
        field: FieldDefinition,
    },
}

impl fmt::Display for AssetChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetChange::ImportAdded(imp) => {
                write!(f, "  + import: {}", imp.object_name)
            }
            AssetChange::ImportRemoved(imp) => {
                write!(f, "  - import: {}", imp.object_name)
            }
            AssetChange::ExportAdded { name, class } => {
                write!(f, "  + {} ({})", name, class)
            }
            AssetChange::ExportRemoved { name, class } => {
                write!(f, "  - {} ({})", name, class)
            }
            AssetChange::PropertyChanged {
                export_name,
                property_path,
                old_value,
                new_value,
            } => {
                write!(
                    f,
                    "  [{}] ~ {}: {} -> {}",
                    export_name, property_path, old_value, new_value
                )
            }
            AssetChange::PropertyAdded {
                export_name,
                property_name,
                value,
            } => {
                write!(f, "  [{}] + {}: {}", export_name, property_name, value)
            }
            AssetChange::PropertyRemoved {
                export_name,
                property_name,
                value,
            } => {
                write!(f, "  [{}] - {}: {}", export_name, property_name, value)
            }
            AssetChange::ExportDataChanged {
                export_name,
                description,
            } => {
                write!(f, "  [{}] ~ {}", export_name, description)
            }
            AssetChange::FieldAdded { export_name, field } => {
                write!(f, "  [{}] + variable: {}", export_name, field)
            }
            AssetChange::FieldRemoved { export_name, field } => {
                write!(f, "  [{}] - variable: {}", export_name, field)
            }
        }
    }
}

/// Compare two structured assets and return a list of semantic changes.
pub fn diff_assets(old: &StructuredAsset, new: &StructuredAsset) -> Vec<AssetChange> {
    let mut changes = Vec::new();

    // 1. Diff imports by object_name.
    diff_imports(&old.imports, &new.imports, &mut changes);

    // 2. Diff exports by object_name.
    diff_exports(&old.exports, &new.exports, &mut changes);

    changes
}

fn diff_imports(old: &[ImportInfo], new: &[ImportInfo], changes: &mut Vec<AssetChange>) {
    let old_map: BTreeMap<&str, &ImportInfo> =
        old.iter().map(|i| (i.object_name.as_str(), i)).collect();
    let new_map: BTreeMap<&str, &ImportInfo> =
        new.iter().map(|i| (i.object_name.as_str(), i)).collect();

    for (name, imp) in &old_map {
        if !new_map.contains_key(name) {
            changes.push(AssetChange::ImportRemoved((*imp).clone()));
        }
    }

    for (name, imp) in &new_map {
        if !old_map.contains_key(name) {
            changes.push(AssetChange::ImportAdded((*imp).clone()));
        }
    }
}

fn diff_exports(old: &[ExportInfo], new: &[ExportInfo], changes: &mut Vec<AssetChange>) {
    let old_map: BTreeMap<&str, &ExportInfo> =
        old.iter().map(|e| (e.object_name.as_str(), e)).collect();
    let new_map: BTreeMap<&str, &ExportInfo> =
        new.iter().map(|e| (e.object_name.as_str(), e)).collect();

    // Removed exports.
    for (name, exp) in &old_map {
        if !new_map.contains_key(name) {
            changes.push(AssetChange::ExportRemoved {
                name: name.to_string(),
                class: exp.class_name.clone(),
            });
        }
    }

    // Added exports.
    for (name, exp) in &new_map {
        if !old_map.contains_key(name) {
            changes.push(AssetChange::ExportAdded {
                name: name.to_string(),
                class: exp.class_name.clone(),
            });
        }
    }

    // Modified exports — compare properties.
    for (name, old_exp) in &old_map {
        if let Some(new_exp) = new_map.get(name) {
            // Compare properties if both parsed successfully.
            match (&old_exp.properties, &new_exp.properties) {
                (Some(old_props), Some(new_props)) => {
                    diff_properties(name, old_props, new_props, changes);
                }
                (Some(_), None) | (None, Some(_)) => {
                    changes.push(AssetChange::ExportDataChanged {
                        export_name: name.to_string(),
                        description: "property parsing changed between versions".to_string(),
                    });
                }
                (None, None) => {
                    // Both unparseable — compare sizes.
                    if old_exp.serial_size != new_exp.serial_size {
                        changes.push(AssetChange::ExportDataChanged {
                            export_name: name.to_string(),
                            description: format!(
                                "binary data changed ({} -> {} bytes)",
                                old_exp.serial_size, new_exp.serial_size
                            ),
                        });
                    }
                }
            }

            // Compare trailing data size (only if no field definitions to show).
            let has_field_changes = match (&old_exp.field_definitions, &new_exp.field_definitions) {
                (Some(old_fields), Some(new_fields)) => old_fields != new_fields,
                (None, Some(_)) | (Some(_), None) => true,
                (None, None) => false,
            };

            if old_exp.trailing_data_size != new_exp.trailing_data_size && !has_field_changes {
                changes.push(AssetChange::ExportDataChanged {
                    export_name: name.to_string(),
                    description: format!(
                        "native data changed ({} -> {} bytes)",
                        old_exp.trailing_data_size, new_exp.trailing_data_size
                    ),
                });
            }

            // Compare field definitions (variable/property definitions in class/struct exports).
            diff_field_definitions(name, &old_exp.field_definitions, &new_exp.field_definitions, changes);
        }
    }
}

fn diff_field_definitions(
    export_name: &str,
    old_fields: &Option<Vec<FieldDefinition>>,
    new_fields: &Option<Vec<FieldDefinition>>,
    changes: &mut Vec<AssetChange>,
) {
    let (old_f, new_f) = match (old_fields, new_fields) {
        (Some(o), Some(n)) => (o.as_slice(), n.as_slice()),
        (None, Some(n)) => {
            // All fields are new.
            for f in n {
                changes.push(AssetChange::FieldAdded {
                    export_name: export_name.to_string(),
                    field: f.clone(),
                });
            }
            return;
        }
        (Some(o), None) => {
            // All fields were removed.
            for f in o {
                changes.push(AssetChange::FieldRemoved {
                    export_name: export_name.to_string(),
                    field: f.clone(),
                });
            }
            return;
        }
        (None, None) => return,
    };

    // Build maps by field name.
    let old_map: BTreeMap<&str, &FieldDefinition> = old_f.iter()
        .map(|f| (f.field_name.as_str(), f))
        .collect();
    let new_map: BTreeMap<&str, &FieldDefinition> = new_f.iter()
        .map(|f| (f.field_name.as_str(), f))
        .collect();

    // Removed fields.
    for (name, field) in &old_map {
        if !new_map.contains_key(name) {
            changes.push(AssetChange::FieldRemoved {
                export_name: export_name.to_string(),
                field: (*field).clone(),
            });
        }
    }

    // Added fields.
    for (name, field) in &new_map {
        if !old_map.contains_key(name) {
            changes.push(AssetChange::FieldAdded {
                export_name: export_name.to_string(),
                field: (*field).clone(),
            });
        }
    }
}

fn diff_properties(
    export_name: &str,
    old_props: &[TaggedProperty],
    new_props: &[TaggedProperty],
    changes: &mut Vec<AssetChange>,
) {
    // Build maps keyed by (name, array_index).
    let old_map: BTreeMap<(&str, u32), &TaggedProperty> = old_props
        .iter()
        .map(|p| ((p.name.as_str(), p.array_index), p))
        .collect();
    let new_map: BTreeMap<(&str, u32), &TaggedProperty> = new_props
        .iter()
        .map(|p| ((p.name.as_str(), p.array_index), p))
        .collect();

    // Removed properties.
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

    // Added properties.
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

    // Changed properties.
    for ((name, idx), old_prop) in &old_map {
        if let Some(new_prop) = new_map.get(&(*name, *idx)) {
            if old_prop.value != new_prop.value {
                let prop_path = if *idx > 0 {
                    format!("{}[{}]", name, idx)
                } else {
                    name.to_string()
                };

                // For structs, recurse to show field-level diffs.
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
