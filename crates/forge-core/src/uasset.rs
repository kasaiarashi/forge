// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Lightweight .uasset/.umap metadata extraction using header-only parsing.
//! No full deserialization — fast enough for on-demand use in web UI and CLI.
//!
//! Powered by the `uasset` crate by Jørgen P. Tjernø (MIT / Apache-2.0).
//! Original: <https://github.com/jorgenpt/uasset-rs>

use std::io::Cursor;
use uasset::{AssetHeader, ObjectReference, PackageFlags};

/// Metadata extracted from a UE asset header.
#[derive(Debug, Clone)]
pub struct AssetMetadata {
    /// Primary asset class (e.g. "StaticMesh", "Texture2D", "Blueprint")
    pub asset_class: String,
    /// Engine version the asset was saved with (e.g. "5.7.0")
    pub engine_version: String,
    /// Package flags (e.g. "Cooked", "EditorOnly", "ContainsMap")
    pub package_flags: Vec<String>,
    /// Imported package dependencies
    pub dependencies: Vec<String>,
}

/// Parse uasset/umap header and extract metadata.
/// Returns `None` for non-UE assets or on any parse error.
pub fn parse_uasset(data: &[u8]) -> Option<AssetMetadata> {
    let cursor = Cursor::new(data);
    let header = match AssetHeader::new(cursor) {
        Ok(h) => h,
        Err(e) => {
            tracing::debug!("uasset parse failed: {}", e);
            return None;
        }
    };

    // Extract the primary asset class from the first export's class reference.
    // The export's class() returns an ObjectReference pointing to an import,
    // whose object_name we can resolve to get the class name.
    let asset_class = if let Some(export) = header.exports.first() {
        match export.class() {
            ObjectReference::Import { import_index } => {
                if let Some(import) = header.imports.get(import_index) {
                    header
                        .resolve_name(&import.object_name)
                        .map(|s| s.to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                }
            }
            _ => {
                // If class is an export reference, try the export's own object_name
                header
                    .resolve_name(&export.object_name)
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            }
        }
    } else {
        String::new()
    };

    // Format engine version.
    let ev = &header.engine_version;
    let engine_version = if ev.is_empty() {
        String::new()
    } else {
        format!("{}.{}.{}", ev.major, ev.minor, ev.patch)
    };

    // Decode package flags.
    let flags_raw = header.package_flags;
    let mut package_flags = Vec::new();
    let flag_checks: &[(u32, &str)] = &[
        (PackageFlags::Cooked as u32, "Cooked"),
        (PackageFlags::EditorOnly as u32, "EditorOnly"),
        (PackageFlags::ContainsMap as u32, "ContainsMap"),
        (PackageFlags::ContainsMapData as u32, "ContainsMapData"),
        (PackageFlags::ContainsScript as u32, "ContainsScript"),
        (PackageFlags::ServerSideOnly as u32, "ServerSideOnly"),
        (PackageFlags::ClientOptional as u32, "ClientOptional"),
        (PackageFlags::FilterEditorOnly as u32, "FilterEditorOnly"),
        (PackageFlags::Developer as u32, "Developer"),
        (PackageFlags::UncookedOnly as u32, "UncookedOnly"),
        (PackageFlags::CompiledIn as u32, "CompiledIn"),
        (PackageFlags::ContainsNoAsset as u32, "ContainsNoAsset"),
    ];
    for &(flag, name) in flag_checks {
        if flags_raw & flag != 0 {
            package_flags.push(name.to_string());
        }
    }

    // Collect imported package dependencies.
    let dependencies: Vec<String> = header
        .package_import_iter()
        .map(|s| s.to_string())
        .collect();

    Some(AssetMetadata {
        asset_class,
        engine_version,
        package_flags,
        dependencies,
    })
}

/// Check if a file path is a UE asset (by extension).
pub fn is_uasset_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".uasset") || lower.ends_with(".umap")
}
