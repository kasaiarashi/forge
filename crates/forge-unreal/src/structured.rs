//! Top-level "everything resolved" view of a `.uasset`. Used by Forge's diff
//! and chunking layers as a stable, serde-friendly tree.

use serde::{Deserialize, Serialize};
use std::io::Cursor;

use crate::ffield::{self, FieldDefinition};
use crate::property::{self, TaggedProperty};
use crate::{AssetHeader, ObjectReference, PackageFlags};

/// Fully decoded asset summary: header metadata + resolved import / export
/// names + parsed properties + scanned field definitions.
#[derive(Debug, Serialize, Deserialize)]
pub struct StructuredAsset {
    /// `"{major}.{minor}.{patch}"` from the embedded engine version.
    pub engine_version: String,
    pub package_flags: u32,
    pub names: Vec<String>,
    pub imports: Vec<ImportInfo>,
    pub exports: Vec<ExportInfo>,
    /// Non-fatal issues encountered during parsing (e.g. cooked-asset warnings).
    pub parse_warnings: Vec<String>,
}

/// Resolved import row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportInfo {
    pub index: usize,
    pub class_package: String,
    pub class_name: String,
    pub object_name: String,
    pub outer_name: Option<String>,
}

/// Resolved export row, optionally with parsed properties and field defs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportInfo {
    pub index: usize,
    pub object_name: String,
    pub class_name: String,
    pub serial_size: i64,
    /// Absolute offset into the combined `.uasset + .uexp` byte stream.
    pub serial_offset: i64,
    pub outer_name: Option<String>,
    pub properties: Option<Vec<TaggedProperty>>,
    pub field_definitions: Option<Vec<FieldDefinition>>,
    pub trailing_data_size: usize,
}

/// Errors raised by the top-level `parse_structured*` functions.
#[derive(Debug)]
pub enum StructuredParseError {
    HeaderParseFailed(String),
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

/// Parse a `.uasset` (header + inline payload) into a structured tree.
pub fn parse_structured(data: &[u8]) -> Result<StructuredAsset, StructuredParseError> {
    parse_structured_with_uexp(data, None)
}

/// Parse a `.uasset` together with its optional `.uexp` sidecar.
///
/// UE splits export payloads into the `.uexp` file; export `serial_offset`
/// values are relative to the notional `header + uexp` concatenation. The
/// concatenation is performed here so property parsing can resolve those
/// offsets against a single byte buffer.
pub fn parse_structured_with_uexp(
    header_data: &[u8],
    uexp_data: Option<&[u8]>,
) -> Result<StructuredAsset, StructuredParseError> {
    let header = AssetHeader::new(Cursor::new(header_data))
        .map_err(|e| StructuredParseError::HeaderParseFailed(e.to_string()))?;

    let cooked = (header.package_flags & PackageFlags::UnversionedProperties as u32) != 0;

    let engine_version = format!(
        "{}.{}.{}",
        header.engine_version.major, header.engine_version.minor, header.engine_version.patch
    );

    let mut warnings: Vec<String> = Vec::new();
    if cooked {
        warnings.push(
            "Asset uses unversioned properties (cooked) — property parsing skipped".to_string(),
        );
    }

    // Concatenate header + uexp so absolute serial offsets are valid against a
    // single buffer. If no uexp was provided we just use the header bytes.
    let combined: Vec<u8>;
    let file_data: &[u8] = match uexp_data {
        Some(uexp) if !uexp.is_empty() => {
            combined = {
                let mut v = Vec::with_capacity(header_data.len() + uexp.len());
                v.extend_from_slice(header_data);
                v.extend_from_slice(uexp);
                v
            };
            &combined
        }
        _ => header_data,
    };

    let imports = build_import_infos(&header);
    let exports = build_export_infos(&header, file_data, cooked, &mut warnings);

    Ok(StructuredAsset {
        engine_version,
        package_flags: header.package_flags,
        names: header.names.clone(),
        imports,
        exports,
        parse_warnings: warnings,
    })
}

fn build_import_infos<R>(header: &AssetHeader<R>) -> Vec<ImportInfo> {
    header
        .imports
        .iter()
        .enumerate()
        .map(|(idx, imp)| {
            let class_package = name_or_placeholder(header, &imp.class_package);
            let class_name = name_or_placeholder(header, &imp.class_name);
            let object_name = name_or_placeholder(header, &imp.object_name);

            let outer_name = match imp.outer() {
                ObjectReference::Import { import_index } => header
                    .imports
                    .get(import_index)
                    .and_then(|o| header.resolve_name(&o.object_name).ok().map(|s| s.into_owned())),
                ObjectReference::Export { export_index } => header
                    .exports
                    .get(export_index)
                    .and_then(|o| header.resolve_name(&o.object_name).ok().map(|s| s.into_owned())),
                ObjectReference::None => None,
            };

            ImportInfo {
                index: idx,
                class_package,
                class_name,
                object_name,
                outer_name,
            }
        })
        .collect()
}

fn build_export_infos<R>(
    header: &AssetHeader<R>,
    file_data: &[u8],
    cooked: bool,
    warnings: &mut Vec<String>,
) -> Vec<ExportInfo> {
    header
        .exports
        .iter()
        .enumerate()
        .map(|(idx, exp)| {
            let object_name = name_or_placeholder(header, &exp.object_name);

            let class_name = match exp.class() {
                ObjectReference::Import { import_index } => header
                    .imports
                    .get(import_index)
                    .map(|i| name_or_placeholder(header, &i.object_name))
                    .unwrap_or_else(|| "???".into()),
                ObjectReference::Export { export_index } => header
                    .exports
                    .get(export_index)
                    .map(|e| name_or_placeholder(header, &e.object_name))
                    .unwrap_or_else(|| "???".into()),
                ObjectReference::None => "Class".into(),
            };

            let outer_name = match exp.outer() {
                ObjectReference::Export { export_index } => header
                    .exports
                    .get(export_index)
                    .and_then(|o| header.resolve_name(&o.object_name).ok().map(|s| s.into_owned())),
                ObjectReference::Import { import_index } => header
                    .imports
                    .get(import_index)
                    .and_then(|o| header.resolve_name(&o.object_name).ok().map(|s| s.into_owned())),
                ObjectReference::None => None,
            };

            // Cooked assets don't carry tag headers so we can't decode their
            // properties without a UE schema — return shape-only metadata.
            if cooked {
                return ExportInfo {
                    index: idx,
                    object_name,
                    class_name,
                    serial_size: exp.serial_size,
                    serial_offset: exp.serial_offset,
                    outer_name,
                    properties: None,
                    field_definitions: None,
                    trailing_data_size: exp.serial_size as usize,
                };
            }

            let (properties, trailing_data_size) =
                decode_export_properties(file_data, exp, &header.names, warnings);

            let field_definitions = {
                let off = exp.serial_offset as usize;
                let len = exp.serial_size as usize;
                if off + len <= file_data.len() {
                    ffield::parse_field_definitions(&file_data[off..off + len], &header.names, &class_name)
                } else {
                    None
                }
            };

            ExportInfo {
                index: idx,
                object_name,
                class_name,
                serial_size: exp.serial_size,
                serial_offset: exp.serial_offset,
                outer_name,
                properties,
                field_definitions,
                trailing_data_size,
            }
        })
        .collect()
}

fn name_or_placeholder<R>(header: &AssetHeader<R>, name: &crate::NameReference) -> String {
    header
        .resolve_name(name)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| "???".to_string())
}

/// Slice an export's tagged-property region out of `file_data` and parse it.
/// Returns `(parsed_properties, trailing_native_size)`.
fn decode_export_properties(
    file_data: &[u8],
    export: &crate::ObjectExport,
    names: &[String],
    warnings: &mut Vec<String>,
) -> (Option<Vec<TaggedProperty>>, usize) {
    let serial_offset = export.serial_offset as usize;
    let serial_size = export.serial_size as usize;

    if serial_offset + serial_size > file_data.len() {
        // Property bytes live in a `.uexp` we weren't given.
        return (None, 0);
    }

    // Prefer the explicit script-serialization range (UE5+); otherwise treat
    // the entire export as the property region.
    let prop_start_rel = export.script_serialization_start_offset;
    let prop_end_rel = export.script_serialization_end_offset;

    let (prop_start, prop_end) = if prop_start_rel >= 0 && prop_end_rel > prop_start_rel {
        let s = serial_offset + prop_start_rel as usize;
        let e = serial_offset + prop_end_rel as usize;
        if e <= file_data.len() {
            (s, e)
        } else {
            (serial_offset, serial_offset + serial_size)
        }
    } else {
        (serial_offset, serial_offset + serial_size)
    };

    let region = &file_data[prop_start..prop_end];
    match property::parse_tagged_properties(region, names) {
        Ok(props) => {
            let trailing = serial_size.saturating_sub(prop_end - serial_offset);
            (Some(props), trailing)
        }
        Err(e) => {
            let label = names
                .get(export.object_name.index as usize)
                .cloned()
                .unwrap_or_else(|| "Export[?]".to_string());
            warnings.push(format!("Failed to parse properties for '{}': {}", label, e));
            (None, serial_size)
        }
    }
}

/// Heuristic scan that pulls Blueprint variable `(name, type)` pairs out of
/// `FBPVariableDescription` runs. Used by diff tools to enrich Blueprint
/// reports with named variable additions / removals.
pub fn scan_blueprint_variables(data: &[u8], names: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();

    let varname_idx = match names.iter().position(|n| n == "VarName") {
        Some(i) => i,
        None => return out,
    };
    let nameprop_idx = match names.iter().position(|n| n == "NameProperty") {
        Some(i) => i,
        None => return out,
    };

    let varname_bytes = (varname_idx as u32).to_le_bytes();
    let nameprop_bytes = (nameprop_idx as u32).to_le_bytes();
    let zero_bytes = 0u32.to_le_bytes();

    let pin_category_idx = names.iter().position(|n| n == "PinCategory");

    if data.len() < 24 {
        return out;
    }

    for off in 0..data.len().saturating_sub(24) {
        // Check the two FName headers ("VarName" + "NameProperty") sit back to
        // back with their `number` fields zeroed.
        if data[off..off + 4] != varname_bytes
            || data[off + 4..off + 8] != zero_bytes
            || data[off + 8..off + 12] != nameprop_bytes
            || data[off + 12..off + 16] != zero_bytes
        {
            continue;
        }

        // Tag header is 9 bytes (size i32 + array_index i32 + has_prop_guid u8),
        // so the FName payload starts at +25.
        let payload = off + 25;
        if payload + 8 > data.len() {
            continue;
        }

        let Ok(name_idx_b) = data[payload..payload + 4].try_into() else { continue };
        let Ok(name_num_b) = data[payload + 4..payload + 8].try_into() else { continue };
        let name_idx = u32::from_le_bytes(name_idx_b) as usize;
        let name_num = u32::from_le_bytes(name_num_b);

        let Some(base) = names.get(name_idx) else { continue };
        let mut var_name = base.clone();
        if name_num > 0 {
            var_name.push('_');
            var_name.push_str(&(name_num - 1).to_string());
        }

        let var_type = pin_category_idx
            .and_then(|pc| find_pin_category(data, payload + 8, pc, names))
            .unwrap_or_else(|| "Variable".to_string());

        out.push((var_name, var_type));
    }
    out
}

fn find_pin_category(
    data: &[u8],
    start: usize,
    pin_category_name_idx: usize,
    names: &[String],
) -> Option<String> {
    let pin_bytes = (pin_category_name_idx as u32).to_le_bytes();
    let zero = 0u32.to_le_bytes();

    let scan_end = (start + 200).min(data.len().saturating_sub(24));
    for pos in start..scan_end {
        if data[pos..pos + 4] != pin_bytes || data[pos + 4..pos + 8] != zero {
            continue;
        }
        let Ok(type_idx_b) = data[pos + 8..pos + 12].try_into() else { continue };
        let type_idx = u32::from_le_bytes(type_idx_b) as usize;
        if names.get(type_idx).map(|s| s.as_str()) != Some("NameProperty") {
            continue;
        }
        let cat_pos = pos + 24;
        if cat_pos + 4 > data.len() {
            continue;
        }
        let Ok(cat_idx_b) = data[cat_pos..cat_pos + 4].try_into() else { continue };
        let cat_idx = u32::from_le_bytes(cat_idx_b) as usize;
        if let Some(cat) = names.get(cat_idx) {
            return Some(pin_category_to_type(cat));
        }
    }
    None
}

/// Map a Blueprint pin category string to the equivalent UE type label.
pub fn pin_category_to_type(category: &str) -> String {
    match category {
        "bool" => "bool",
        "byte" => "byte",
        "int" => "int32",
        "int64" => "int64",
        "real" | "float" => "float",
        "double" => "double",
        "string" => "FString",
        "name" => "FName",
        "text" => "FText",
        "object" | "class" => "Object",
        "struct" => "Struct",
        "enum" => "Enum",
        other => other,
    }
    .to_string()
}
