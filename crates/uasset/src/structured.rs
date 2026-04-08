//! Structured asset representation for diffing and display.
//!
//! Combines header parsing with property parsing into a single representation
//! that captures all semantically meaningful parts of a `.uasset` file.

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
}

/// An export object with optional parsed properties.
#[derive(Debug, Clone)]
pub struct ExportInfo {
    pub index: usize,
    pub object_name: String,
    pub class_name: String,
    pub serial_size: i64,
    /// Parsed tagged properties (None if parsing failed or was skipped).
    pub properties: Option<Vec<TaggedProperty>>,
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
            ImportInfo {
                index: i,
                class_package,
                class_name,
                object_name,
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

            // For cooked assets, skip property parsing but still record export metadata.
            if is_cooked {
                return ExportInfo {
                    index: i,
                    object_name,
                    class_name,
                    serial_size: exp.serial_size,
                    properties: None,
                    trailing_data_size: exp.serial_size as usize,
                };
            }

            // Try to parse tagged properties from the export data region.
            let (properties, trailing_data_size) =
                parse_export_properties(file_data, exp, &header.names, &mut warnings);

            ExportInfo {
                index: i,
                object_name,
                class_name,
                serial_size: exp.serial_size,
                properties,
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
