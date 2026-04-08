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
    /// An enumerator was added to a UserDefinedEnum.
    EnumValueAdded {
        export_name: String,
        value_name: String,
        display_name: Option<String>,
    },
    /// An enumerator was removed from a UserDefinedEnum.
    EnumValueRemoved {
        export_name: String,
        value_name: String,
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
            AssetChange::EnumValueAdded { export_name, value_name, display_name } => {
                if let Some(dn) = display_name {
                    write!(f, "  [{}] + enum: {} ({})", export_name, value_name, dn)
                } else {
                    write!(f, "  [{}] + enum: {}", export_name, value_name)
                }
            }
            AssetChange::EnumValueRemoved { export_name, value_name } => {
                write!(f, "  [{}] - enum: {}", export_name, value_name)
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
    diff_assets_with_data(old, None, new, None)
}

/// Compare two structured assets with optional raw file data for deep scanning.
///
/// When raw data is provided, Blueprint exports are scanned for `NewVariables`
/// to detect added/removed Blueprint variables (e.g., "TestVar (bool)").
pub fn diff_assets_with_data(
    old: &StructuredAsset,
    old_data: Option<&[u8]>,
    new: &StructuredAsset,
    new_data: Option<&[u8]>,
) -> Vec<AssetChange> {
    let mut changes = Vec::new();

    // 1. Diff imports by object_name.
    diff_imports(&old.imports, &new.imports, &mut changes);

    // 2. Diff exports by object_name.
    diff_exports(&old.exports, &new.exports, &mut changes);

    // 3. Diff Blueprint variables from NewVariables tagged property.
    diff_blueprint_variables(&old.exports, old_data, &old.names,
                             &new.exports, new_data, &new.names,
                             &mut changes);

    // 4. Diff UserDefinedEnum enumerators via name table comparison.
    diff_enum_values(&old.exports, &old.names, old_data,
                     &new.exports, &new.names, new_data, &mut changes);

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

/// Diff Blueprint variables from the `NewVariables` tagged property on UBlueprint exports.
///
/// Blueprint user-created variables (like "TestVar") are stored as
/// `FBPVariableDescription` entries in the `NewVariables` array property
/// on the UBlueprint export, NOT in the BlueprintGeneratedClass's ChildProperties.
fn diff_blueprint_variables(
    old_exports: &[ExportInfo],
    old_data: Option<&[u8]>,
    old_names: &[String],
    new_exports: &[ExportInfo],
    new_data: Option<&[u8]>,
    new_names: &[String],
    changes: &mut Vec<AssetChange>,
) {
    let old_map: BTreeMap<&str, &ExportInfo> =
        old_exports.iter().map(|e| (e.object_name.as_str(), e)).collect();
    let new_map: BTreeMap<&str, &ExportInfo> =
        new_exports.iter().map(|e| (e.object_name.as_str(), e)).collect();

    for (name, new_exp) in &new_map {
        if !is_blueprint_export(&new_exp.class_name) {
            continue;
        }

        let new_vars = extract_blueprint_var_names_with_scan(new_exp, new_data, new_names);
        let old_vars = old_map
            .get(name)
            .map(|e| extract_blueprint_var_names_with_scan(e, old_data, old_names))
            .unwrap_or_default();

        if new_vars == old_vars {
            continue;
        }

        // Find the BlueprintGeneratedClass name for display — it's usually the export
        // name + "_C" suffix, but we use the Blueprint name itself as context.
        let display_name = format!("{}_C", name);

        // Detect added variables.
        for (var_name, var_type) in &new_vars {
            if !old_vars.iter().any(|(n, _)| n == var_name) {
                changes.push(AssetChange::FieldAdded {
                    export_name: display_name.clone(),
                    field: FieldDefinition {
                        field_type: var_type.clone(),
                        field_name: var_name.clone(),
                        array_dim: 1,
                        property_flags: 0,
                        struct_type: None,
                        inner_type: None,
                        key_type: None,
                        value_type: None,
                    },
                });
            }
        }

        // Detect removed variables.
        for (var_name, var_type) in &old_vars {
            if !new_vars.iter().any(|(n, _)| n == var_name) {
                changes.push(AssetChange::FieldRemoved {
                    export_name: display_name.clone(),
                    field: FieldDefinition {
                        field_type: var_type.clone(),
                        field_name: var_name.clone(),
                        array_dim: 1,
                        property_flags: 0,
                        struct_type: None,
                        inner_type: None,
                        key_type: None,
                        value_type: None,
                    },
                });
            }
        }
    }
}

/// Check if an export is a UBlueprint (stores NewVariables).
fn is_blueprint_export(class_name: &str) -> bool {
    class_name == "Blueprint"
        || class_name == "WidgetBlueprint"
        || class_name == "AnimBlueprint"
        || class_name == "GameplayAbilityBlueprint"
        || class_name.ends_with("Blueprint")
}

/// Extract variable names and types from a Blueprint export.
///
/// Blueprint variables are stored in the `NewVariables` tagged property of the
/// UBlueprint export. Since the tagged property parser may fail on complex Blueprint
/// exports (the data starts with native UObject header), we also scan the raw export
/// data for `VarName` FName patterns within the `NewVariables` section.
///
/// Returns Vec<(var_name, var_type_hint)>.
fn extract_blueprint_var_names(export: &ExportInfo) -> Vec<(String, String)> {
    // First try: use parsed tagged properties if available.
    if let Some(ref props) = export.properties {
        let new_vars_prop = props.iter().find(|p| p.name == "NewVariables");
        if let Some(TaggedProperty {
            value: PropertyValue::Array { elements, .. },
            ..
        }) = new_vars_prop {
            let mut vars = Vec::new();
            for elem in elements {
                if let PropertyValue::Struct { fields, .. } = elem {
                    let var_name = fields.iter()
                        .find(|f| f.name == "VarName")
                        .and_then(|f| match &f.value {
                            PropertyValue::Name(n) => Some(n.clone()),
                            _ => None,
                        });
                    let var_type = extract_var_type_from_fields(fields);
                    if let Some(name) = var_name {
                        vars.push((name, var_type));
                    }
                }
            }
            if !vars.is_empty() {
                return vars;
            }
        }
    }

    // Properties not available from the tagged property parser.
    Vec::new()
}

/// Extract Blueprint variable names using raw data scan as fallback.
fn extract_blueprint_var_names_with_scan(
    export: &ExportInfo,
    file_data: Option<&[u8]>,
    names: &[String],
) -> Vec<(String, String)> {
    // First try parsed properties.
    let from_props = extract_blueprint_var_names(export);
    if !from_props.is_empty() {
        return from_props;
    }

    // Fallback: scan raw export data for NewVariables.
    let data = match file_data {
        Some(d) => d,
        None => return Vec::new(),
    };

    let offset = export.serial_size as usize; // serial_size, not offset — we need serial_offset
    // We can't get serial_offset from ExportInfo. But we can scan the whole file
    // for the NewVariables pattern — it only appears once per Blueprint.
    // Scan the raw file data for VarName FName patterns in the NewVariables region.
    let vars = uasset::structured::scan_blueprint_variables(data, names);
    if !vars.is_empty() {
        return vars;
    }

    Vec::new()
}

/// Extract a human-readable type string from FBPVariableDescription fields.
///
/// The VarType is an FEdGraphPinType struct with PinCategory (Name) that maps to:
/// "bool" -> BoolProperty, "int" -> IntProperty, "real"/"float"/"double" -> FloatProperty,
/// "string" -> StrProperty, "name" -> NameProperty, "text" -> TextProperty,
/// "object" -> ObjectProperty, "struct" -> StructProperty, etc.
fn extract_var_type_from_fields(fields: &[TaggedProperty]) -> String {
    let var_type_field = fields.iter().find(|f| f.name == "VarType");
    if let Some(TaggedProperty { value: PropertyValue::Struct { fields: type_fields, .. }, .. }) = var_type_field {
        // PinCategory is a Name property inside the FEdGraphPinType struct.
        if let Some(cat) = type_fields.iter().find(|f| f.name == "PinCategory") {
            if let PropertyValue::Name(cat_name) = &cat.value {
                return pin_category_to_type(cat_name);
            }
        }
    }

    // Fallback: check for PropertyFlags to guess the type.
    "Variable".to_string()
}

/// Convert UE pin category name to a human-readable type name.
fn pin_category_to_type(category: &str) -> String {
    match category {
        "bool" => "bool".to_string(),
        "byte" => "byte".to_string(),
        "int" => "int32".to_string(),
        "int64" => "int64".to_string(),
        "real" | "float" => "float".to_string(),
        "double" => "double".to_string(),
        "string" => "FString".to_string(),
        "name" => "FName".to_string(),
        "text" => "FText".to_string(),
        "object" | "class" => "Object".to_string(),
        "softobject" | "softclass" => "SoftObject".to_string(),
        "interface" => "Interface".to_string(),
        "struct" => "Struct".to_string(),
        "enum" => "Enum".to_string(),
        "delegate" | "mcdelegate" => "Delegate".to_string(),
        other => other.to_string(),
    }
}

/// Diff UserDefinedEnum enumerators by comparing name table entries.
///
/// UE enum values are stored as FName entries in the name table with the pattern
/// `EnumName::EnumeratorName`. By comparing which enum-prefixed names exist in
/// the old vs new name tables, we can detect added/removed enumerators.
fn diff_enum_values(
    old_exports: &[ExportInfo],
    old_names: &[String],
    old_data: Option<&[u8]>,
    new_exports: &[ExportInfo],
    new_names: &[String],
    new_data: Option<&[u8]>,
    changes: &mut Vec<AssetChange>,
) {
    let old_map: BTreeMap<&str, &ExportInfo> =
        old_exports.iter().map(|e| (e.object_name.as_str(), e)).collect();
    let new_map: BTreeMap<&str, &ExportInfo> =
        new_exports.iter().map(|e| (e.object_name.as_str(), e)).collect();

    for (name, new_exp) in &new_map {
        if !is_enum_export(&new_exp.class_name) {
            continue;
        }

        let prefix = format!("{}::", name);

        // Collect enum values and build display name map.
        let new_values: Vec<&str> = new_names.iter()
            .filter(|n| n.starts_with(&prefix))
            .map(|n| &n[prefix.len()..])
            .collect();

        let old_values: Vec<&str> = if old_map.contains_key(name) {
            old_names.iter()
                .filter(|n| n.starts_with(&prefix))
                .map(|n| &n[prefix.len()..])
                .collect()
        } else {
            Vec::new()
        };

        if new_values == old_values {
            continue;
        }

        // Build display name map from raw file data (display names are inline FText,
        // not in the name table).
        let new_display = new_data
            .map(|d| build_enum_display_map_from_data(name, &new_values, new_names, d))
            .unwrap_or_default();
        let old_display = old_data
            .map(|d| build_enum_display_map_from_data(name, &old_values, old_names, d))
            .unwrap_or_default();

        // Detect added enumerators.
        for val in &new_values {
            if !old_values.contains(val) && !val.ends_with("_MAX") {
                let display = new_display.get(*val).cloned()
                    .unwrap_or_else(|| val.to_string());
                changes.push(AssetChange::EnumValueAdded {
                    export_name: name.to_string(),
                    value_name: display,
                    display_name: None,
                });
            }
        }

        // Detect removed enumerators.
        for val in &old_values {
            if !new_values.contains(val) && !val.ends_with("_MAX") {
                let display = old_display.get(*val).cloned()
                    .unwrap_or_else(|| val.to_string());
                changes.push(AssetChange::EnumValueRemoved {
                    export_name: name.to_string(),
                    value_name: display,
                });
            }
        }
    }
}

fn is_enum_export(class_name: &str) -> bool {
    class_name == "UserDefinedEnum" || class_name == "Enum"
}

/// Build a map from enumerator internal name to display name by scanning raw data.
///
/// Display names are stored as FText values in the DisplayNameMap MapProperty.
/// They appear as inline FStrings in the raw data, in the same order as the
/// enumerators. We collect all display-name-like FStrings from the export data
/// and match them to enumerators by order.
fn build_enum_display_map_from_data(
    _enum_name: &str,
    values: &[&str],
    _names: &[String],
    data: &[u8],
) -> BTreeMap<String, String> {
    let mut display_map = BTreeMap::new();

    // UE keywords and property names to exclude from display name candidates.
    const EXCLUDE: &[&str] = &[
        "None", "Class", "Package", "MapProperty", "NameProperty", "TextProperty",
        "StrProperty", "IntProperty", "BoolProperty", "EnumProperty", "StructProperty",
        "ArrayProperty", "UInt32Property", "ObjectProperty", "ByteProperty",
        "UserDefinedEnum", "BlueprintType", "PackageLocalizationNamespace",
        "UniqueNameIndex", "true", "false", "EnumDescription", "DisplayNameMap",
    ];

    // Collect enumerator display names: readable FStrings that appear after the
    // name table and aren't UE keywords, paths, or enumerator internal names.
    // They appear in the raw data as: i32(length) + ASCII bytes + null terminator.
    let non_max_values: Vec<&&str> = values.iter()
        .filter(|v| !v.ends_with("_MAX"))
        .collect();

    let mut display_names: Vec<String> = Vec::new();

    // Scan the data for readable FStrings.
    let mut off = 0usize;
    while off + 4 < data.len() {
        let Ok(lb) = data[off..off + 4].try_into() else { off += 1; continue };
        let length = i32::from_le_bytes(lb);

        if length >= 3 && length <= 50 {
            let str_start = off + 4;
            let str_end = str_start + length as usize;
            if str_end <= data.len() {
                let bytes = &data[str_start..str_end - 1]; // exclude null terminator
                if !bytes.is_empty()
                    && data[str_end - 1] == 0 // null terminated
                    && bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ')
                {
                    let text = String::from_utf8_lossy(bytes).to_string();
                    // Filter: not a UE keyword, not a path, not an internal enum name.
                    let looks_like_guid = (text.len() == 32
                        && text.chars().all(|c| c.is_ascii_hexdigit()))
                        || (text.starts_with('[') && text.ends_with(']')
                            && text.len() > 20
                            && text[1..text.len()-1].chars().all(|c| c.is_ascii_hexdigit()));
                    if !text.starts_with('/')
                        && !text.starts_with('+')
                        && !text.contains("::")
                        && !text.contains('.')
                        && !text.starts_with("NewEnumerator")
                        && !text.starts_with("E_")
                        && !EXCLUDE.contains(&text.as_str())
                        && !text.contains("_MAX")
                        && !looks_like_guid
                    {
                        // Avoid duplicates — display names appear twice in the file
                        if !display_names.contains(&text) {
                            display_names.push(text);
                        }
                    }
                    off = str_end;
                    continue;
                }
            }
        }
        off += 1;
    }

    // Match display names to enumerators by order.
    for (i, val) in non_max_values.iter().enumerate() {
        if i < display_names.len() {
            display_map.insert(val.to_string(), display_names[i].clone());
        }
    }

    display_map
}
