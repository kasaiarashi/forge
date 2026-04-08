//! Structured asset representation for diffing and display.
//!
//! Combines header parsing with property parsing into a single representation
//! that captures all semantically meaningful parts of a `.uasset` file.

use crate::ffield::{self, FieldDefinition};
use crate::property::{self, TaggedProperty};
use crate::{AssetHeader, ObjectReference, PackageFlags};
use std::io::Cursor;

/// A fully parsed, diffable representation of a `.uasset` file.
#[derive(Debug)]
pub struct StructuredAsset {
    /// Engine version string (e.g., "5.4.0").
    pub engine_version: String,
    /// Raw package flags.
    pub package_flags: u32,
    /// The name table.
    pub names: Vec<String>,
    /// Resolved import paths.
    pub imports: Vec<ImportInfo>,
    /// Export objects with parsed properties.
    pub exports: Vec<ExportInfo>,
    /// Non-fatal warnings during parsing.
    pub parse_warnings: Vec<String>,
}

/// A resolved import dependency.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportInfo {
    pub index: usize,
    pub class_package: String,
    pub class_name: String,
    pub object_name: String,
    /// Resolved name of the outer (parent) object, if any.
    pub outer_name: Option<String>,
}

/// An export object with optional parsed properties.
#[derive(Debug, Clone)]
pub struct ExportInfo {
    pub index: usize,
    pub object_name: String,
    pub class_name: String,
    pub serial_size: i64,
    /// Resolved name of the outer (parent) object, if any.
    pub outer_name: Option<String>,
    /// Parsed tagged properties (None if parsing failed or was skipped).
    pub properties: Option<Vec<TaggedProperty>>,
    /// Parsed field/property definitions for class/struct exports.
    /// Present when this export is a UClass/UStruct that defines properties (e.g., BlueprintGeneratedClass).
    pub field_definitions: Option<Vec<FieldDefinition>>,
    /// Size of trailing native data after the property list.
    pub trailing_data_size: usize,
}

/// Errors during structured asset parsing.
#[derive(Debug)]
pub enum StructuredParseError {
    /// The asset header couldn't be parsed at all.
    HeaderParseFailed(String),
    /// The asset uses unversioned properties (needs UE schema).
    UnversionedProperties,
}

impl std::fmt::Display for StructuredParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StructuredParseError::HeaderParseFailed(e) => write!(f, "header parse failed: {}", e),
            StructuredParseError::UnversionedProperties => {
                write!(f, "asset uses unversioned properties (cooked asset)")
            }
        }
    }
}

impl std::error::Error for StructuredParseError {}

/// Parse a `.uasset` file into a structured representation.
///
/// Returns an error if the header can't be parsed or the asset uses unversioned properties
/// (unless in header-only mode for cooked assets).
/// Individual export parse failures are recorded as warnings, not errors.
pub fn parse_structured(data: &[u8]) -> Result<StructuredAsset, StructuredParseError> {
    parse_structured_with_uexp(data, None)
}

/// Parse a `.uasset` file with optional `.uexp` continuation data.
///
/// When UE splits an asset, export data beyond the header file lives in the `.uexp` file.
/// The `.uexp` data logically continues at the header's `total_header_size` offset.
/// Pass it here so property parsing can access all export data.
pub fn parse_structured_with_uexp(
    header_data: &[u8],
    uexp_data: Option<&[u8]>,
) -> Result<StructuredAsset, StructuredParseError> {
    let cursor = Cursor::new(header_data);
    let header = AssetHeader::new(cursor)
        .map_err(|e| StructuredParseError::HeaderParseFailed(format!("{}", e)))?;

    // Check for unversioned properties — we can't parse property values without UE
    // class schemas, but we can still return header-level information (imports, exports,
    // sizes) which is useful for diffing and semantic chunking.
    let is_cooked =
        header.package_flags & (PackageFlags::UnversionedProperties as u32) != 0;

    let engine_version = format!(
        "{}.{}.{}",
        header.engine_version.major,
        header.engine_version.minor,
        header.engine_version.patch
    );

    let mut warnings = Vec::new();

    if is_cooked {
        warnings.push("Asset uses unversioned properties (cooked) — property parsing skipped".to_string());
    }

    // Build combined data view: header + uexp continuation.
    let combined_data: Vec<u8>;
    let file_data: &[u8] = match uexp_data {
        Some(uexp) if !uexp.is_empty() => {
            combined_data = {
                let mut v = Vec::with_capacity(header_data.len() + uexp.len());
                v.extend_from_slice(header_data);
                v.extend_from_slice(uexp);
                v
            };
            &combined_data
        }
        _ => header_data,
    };

    // Resolve imports to readable strings.
    let imports: Vec<ImportInfo> = header
        .imports
        .iter()
        .enumerate()
        .map(|(i, imp)| {
            let class_package = header
                .resolve_name(&imp.class_package)
                .unwrap_or_else(|_| "???".into())
                .into_owned();
            let class_name = header
                .resolve_name(&imp.class_name)
                .unwrap_or_else(|_| "???".into())
                .into_owned();
            let object_name = header
                .resolve_name(&imp.object_name)
                .unwrap_or_else(|_| "???".into())
                .into_owned();
            let outer_name = match imp.outer() {
                ObjectReference::Import { import_index } => {
                    header.imports.get(import_index).and_then(|o| {
                        header.resolve_name(&o.object_name).ok().map(|s| s.into_owned())
                    })
                }
                ObjectReference::Export { export_index } => {
                    header.exports.get(export_index).and_then(|o| {
                        header.resolve_name(&o.object_name).ok().map(|s| s.into_owned())
                    })
                }
                ObjectReference::None => None,
            };
            ImportInfo {
                index: i,
                class_package,
                class_name,
                object_name,
                outer_name,
            }
        })
        .collect();

    // Parse each export's properties.
    let exports: Vec<ExportInfo> = header
        .exports
        .iter()
        .enumerate()
        .map(|(i, exp)| {
            let object_name = header
                .resolve_name(&exp.object_name)
                .unwrap_or_else(|_| "???".into())
                .into_owned();

            // Resolve class name from class_index.
            let class_name = match exp.class() {
                ObjectReference::Import { import_index } => {
                    if import_index < header.imports.len() {
                        header
                            .resolve_name(&header.imports[import_index].object_name)
                            .unwrap_or_else(|_| "???".into())
                            .into_owned()
                    } else {
                        "???".to_string()
                    }
                }
                ObjectReference::Export { export_index } => {
                    if export_index < header.exports.len() {
                        header
                            .resolve_name(&header.exports[export_index].object_name)
                            .unwrap_or_else(|_| "???".into())
                            .into_owned()
                    } else {
                        "???".to_string()
                    }
                }
                ObjectReference::None => "Class".to_string(),
            };

            // Resolve outer (parent) name.
            let outer_name = match exp.outer() {
                ObjectReference::Export { export_index } => {
                    header.exports.get(export_index).and_then(|o| {
                        header.resolve_name(&o.object_name).ok().map(|s| s.into_owned())
                    })
                }
                ObjectReference::Import { import_index } => {
                    header.imports.get(import_index).and_then(|o| {
                        header.resolve_name(&o.object_name).ok().map(|s| s.into_owned())
                    })
                }
                ObjectReference::None => None,
            };

            // For cooked assets, skip property parsing but still record export metadata.
            if is_cooked {
                return ExportInfo {
                    index: i,
                    object_name,
                    class_name,
                    serial_size: exp.serial_size,
                    outer_name,
                    properties: None,
                    field_definitions: None,
                    trailing_data_size: exp.serial_size as usize,
                };
            }

            // Try to parse tagged properties from the export data region.
            let (properties, trailing_data_size) =
                parse_export_properties(file_data, exp, &header.names, &mut warnings);

            // Try to parse field definitions for class/struct exports.
            let field_definitions = {
                let serial_offset = exp.serial_offset as usize;
                let serial_size = exp.serial_size as usize;
                if serial_offset + serial_size <= file_data.len() {
                    let export_data = &file_data[serial_offset..serial_offset + serial_size];
                    ffield::parse_field_definitions(export_data, &header.names, &class_name)
                } else {
                    None
                }
            };

            ExportInfo {
                index: i,
                object_name,
                class_name,
                serial_size: exp.serial_size,
                outer_name,
                properties,
                field_definitions,
                trailing_data_size,
            }
        })
        .collect();

    Ok(StructuredAsset {
        engine_version,
        package_flags: header.package_flags,
        names: header.names.clone(),
        imports,
        exports,
        parse_warnings: warnings,
    })
}

/// Parse tagged properties from an export's data region.
fn parse_export_properties(
    file_data: &[u8],
    export: &crate::ObjectExport,
    names: &[String],
    warnings: &mut Vec<String>,
) -> (Option<Vec<TaggedProperty>>, usize) {
    let serial_offset = export.serial_offset as usize;
    let serial_size = export.serial_size as usize;

    // Bounds check.
    if serial_offset + serial_size > file_data.len() {
        // Export data is probably in a .uexp file — skip.
        return (None, 0);
    }

    // Determine the property data range within the export.
    let prop_start_rel = export.script_serialization_start_offset;
    let prop_end_rel = export.script_serialization_end_offset;

    let (prop_start, prop_end) = if prop_start_rel >= 0 && prop_end_rel > prop_start_rel {
        // UE5 with explicit script serialization offsets.
        let start = serial_offset + prop_start_rel as usize;
        let end = serial_offset + prop_end_rel as usize;
        if end <= file_data.len() {
            (start, end)
        } else {
            (serial_offset, serial_offset + serial_size)
        }
    } else {
        // Older format: try parsing from the start of the export data.
        (serial_offset, serial_offset + serial_size)
    };

    let prop_data = &file_data[prop_start..prop_end];

    match property::parse_tagged_properties(prop_data, names) {
        Ok(props) => {
            let trailing = serial_size.saturating_sub(prop_end - serial_offset);
            (Some(props), trailing)
        }
        Err(e) => {
            let export_name = if let Some(name) = names.get(export.object_name.index as usize) {
                name.clone()
            } else {
                format!("Export[{}]", 0)
            };
            warnings.push(format!("Failed to parse properties for '{}': {}", export_name, e));
            (None, serial_size)
        }
    }
}


/// Scan file data for Blueprint variable names from the `NewVariables` region.
///
/// Scans for `VarName` + `NameProperty` FName patterns within FBPVariableDescription
/// struct elements. Returns Vec<(var_name, var_type)> for each variable found.
pub fn scan_blueprint_variables(
    data: &[u8],
    names: &[String],
) -> Vec<(String, String)> {
    let mut vars = Vec::new();

    let Some(varname_idx) = names.iter().position(|n| n == "VarName") else { return vars };
    let Some(nameprop_idx) = names.iter().position(|n| n == "NameProperty") else { return vars };

    let vn_bytes = (varname_idx as u32).to_le_bytes();
    let zero = 0u32.to_le_bytes();
    let np_bytes = (nameprop_idx as u32).to_le_bytes();

    let pincat_idx = names.iter().position(|n| n == "PinCategory");

    for offset in 0..data.len().saturating_sub(24) {
        if data[offset..offset + 4] != vn_bytes { continue; }
        if data[offset + 4..offset + 8] != zero { continue; }
        if data[offset + 8..offset + 12] != np_bytes { continue; }
        if data[offset + 12..offset + 16] != zero { continue; }

        // After two FNames (16 bytes): value_size(i32=4) + array_index(i32=4) +
        // has_property_guid(u8=1) = 9 bytes of header. FName value at +25.
        let name_val_offset = offset + 25;
        if name_val_offset + 8 > data.len() { continue; }

        let Ok(ni) = data[name_val_offset..name_val_offset + 4].try_into() else { continue };
        let name_idx = u32::from_le_bytes(ni) as usize;
        let Ok(nn) = data[name_val_offset + 4..name_val_offset + 8].try_into() else { continue };
        let name_num = u32::from_le_bytes(nn);
        if name_idx >= names.len() { continue; }

        let mut var_name = names[name_idx].clone();
        if name_num > 0 {
            var_name.push_str(&format!("_{}", name_num - 1));
        }

        let var_type = if let Some(pc_idx) = pincat_idx {
            find_pin_category_near(data, name_val_offset + 8, pc_idx, names)
                .unwrap_or_else(|| "Variable".to_string())
        } else {
            "Variable".to_string()
        };

        vars.push((var_name, var_type));
    }
    vars
}

fn find_pin_category_near(data: &[u8], start: usize, pincat_name_idx: usize, names: &[String]) -> Option<String> {
    let pc_bytes = (pincat_name_idx as u32).to_le_bytes();
    let zero = 0u32.to_le_bytes();
    let end = (start + 200).min(data.len().saturating_sub(24));

    for off in start..end {
        if data[off..off + 4] != pc_bytes { continue; }
        if data[off + 4..off + 8] != zero { continue; }
        let Ok(ti) = data[off + 8..off + 12].try_into() else { continue };
        let type_idx = u32::from_le_bytes(ti) as usize;
        if type_idx >= names.len() || names[type_idx] != "NameProperty" { continue; }
        let cat_off = off + 24;
        if cat_off + 4 > data.len() { continue; }
        let Ok(ci) = data[cat_off..cat_off + 4].try_into() else { continue };
        let cat_idx = u32::from_le_bytes(ci) as usize;
        if cat_idx < names.len() {
            return Some(pin_category_to_type(&names[cat_idx]));
        }
    }
    None
}

pub fn pin_category_to_type(category: &str) -> String {
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
        "struct" => "Struct".to_string(),
        "enum" => "Enum".to_string(),
        other => other.to_string(),
    }
}
