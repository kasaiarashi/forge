//! Blueprint user-variable diff — reads `NewVariables` on UBlueprint exports.
//!
//! Blueprint user-created variables (like "TestVar") are stored as
//! `FBPVariableDescription` entries in the `NewVariables` array property
//! on the UBlueprint export, NOT in the BlueprintGeneratedClass's ChildProperties.

use std::collections::BTreeMap;
use forge_unreal::ffield::FieldDefinition;
use forge_unreal::property::{PropertyValue, TaggedProperty};
use forge_unreal::structured::ExportInfo;

use super::{AssetDiffHandler, DiffContext};
use crate::change::AssetChange;

pub struct BlueprintVarHandler;

impl AssetDiffHandler for BlueprintVarHandler {
    fn name(&self) -> &'static str { "blueprint_vars" }

    fn diff_top_level(&self, ctx: &DiffContext<'_>, sink: &mut Vec<AssetChange>) {
        diff_blueprint_variables(
            &ctx.old.asset.exports,
            ctx.old.raw_data,
            ctx.old.names(),
            &ctx.new.asset.exports,
            ctx.new.raw_data,
            ctx.new.names(),
            sink,
        );
    }
}

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

        let display_name = format!("{}_C", name);

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

fn is_blueprint_export(class_name: &str) -> bool {
    class_name == "Blueprint"
        || class_name == "WidgetBlueprint"
        || class_name == "AnimBlueprint"
        || class_name == "GameplayAbilityBlueprint"
        || class_name.ends_with("Blueprint")
}

/// First try the parsed tagged-property view. If that yields nothing (as it
/// does on most complex Blueprint exports where the native header defeats
/// the tagged-property parser), fall back to raw byte scanning.
fn extract_blueprint_var_names(export: &ExportInfo) -> Vec<(String, String)> {
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
    Vec::new()
}

fn extract_blueprint_var_names_with_scan(
    export: &ExportInfo,
    file_data: Option<&[u8]>,
    names: &[String],
) -> Vec<(String, String)> {
    let from_props = extract_blueprint_var_names(export);
    if !from_props.is_empty() {
        return from_props;
    }

    let data = match file_data {
        Some(d) => d,
        None => return Vec::new(),
    };

    let vars = forge_unreal::structured::scan_blueprint_variables(data, names);
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
        if let Some(cat) = type_fields.iter().find(|f| f.name == "PinCategory") {
            if let PropertyValue::Name(cat_name) = &cat.value {
                return pin_category_to_type(cat_name);
            }
        }
    }
    "Variable".to_string()
}

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
