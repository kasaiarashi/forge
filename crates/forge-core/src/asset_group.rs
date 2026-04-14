//! Asset group awareness for UE split files.
//!
//! Unreal Engine splits large assets across multiple files:
//! - `.uasset` / `.umap` — header + tables + small export data
//! - `.uexp` — export data continuation (when exports exceed header file)
//! - `.ubulk` — large bulk data (textures, meshes, audio)
//! - `.uptnl` — optional payload data
//!
//! This module treats these as a single logical asset for diffing, merging,
//! and semantic chunking.

use std::path::Path;

/// The set of related files that form a single UE asset.
#[derive(Debug, Clone)]
pub struct AssetGroup {
    /// The .uasset or .umap header file path (always present).
    pub header_path: String,
    /// The .uexp export data file path (optional).
    pub uexp_path: Option<String>,
    /// The .ubulk bulk data file path (optional).
    pub ubulk_path: Option<String>,
    /// The .uptnl optional payload file path (optional).
    pub uptnl_path: Option<String>,
}

/// UE companion file extensions.
const HEADER_EXTENSIONS: &[&str] = &[".uasset", ".umap"];
const COMPANION_EXTENSIONS: &[&str] = &[".uexp", ".ubulk", ".uptnl"];

/// Check if a path is a UE asset header file (.uasset or .umap).
pub fn is_header_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    HEADER_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Check if a path is a UE companion file (.uexp, .ubulk, .uptnl).
pub fn is_companion_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    COMPANION_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Check if a path is any UE asset file (header or companion).
pub fn is_ue_asset_path(path: &str) -> bool {
    is_header_path(path) || is_companion_path(path)
}

/// Given any UE asset file path, resolve all related companion files.
///
/// Returns an `AssetGroup` centered on the header file. If the input is a
/// companion file (.uexp, .ubulk, .uptnl), it finds the header by replacing
/// the extension. UE always names companion files identically except for extension.
pub fn resolve_asset_group(path: &str) -> Option<AssetGroup> {
    let stem = strip_ue_extension(path)?;

    // Determine the header path.
    let header_path = if is_header_path(path) {
        path.to_string()
    } else {
        // Try both .uasset and .umap.
        let uasset = format!("{}.uasset", stem);
        let umap = format!("{}.umap", stem);
        // Prefer .uasset, but the caller can check existence.
        if Path::new(&umap).exists() {
            umap
        } else {
            uasset
        }
    };

    Some(AssetGroup {
        header_path,
        uexp_path: Some(format!("{}.uexp", stem)),
        ubulk_path: Some(format!("{}.ubulk", stem)),
        uptnl_path: Some(format!("{}.uptnl", stem)),
    })
}

/// Given a header path, return the expected companion paths without checking existence.
pub fn companion_paths(header_path: &str) -> Vec<String> {
    let Some(stem) = strip_ue_extension(header_path) else {
        return vec![];
    };
    COMPANION_EXTENSIONS
        .iter()
        .map(|ext| format!("{}{}", stem, ext))
        .collect()
}

/// Find the header path for a companion file.
/// Returns None if the path is not a UE asset file.
pub fn header_for_companion(companion_path: &str) -> Option<String> {
    if !is_companion_path(companion_path) {
        return None;
    }
    let stem = strip_ue_extension(companion_path)?;
    // Try .uasset first (most common), then .umap.
    Some(format!("{}.uasset", stem))
}

/// Strip any UE asset extension and return the stem.
fn strip_ue_extension(path: &str) -> Option<&str> {
    let lower = path.to_lowercase();
    let all_exts: &[&str] = &[".uasset", ".umap", ".uexp", ".ubulk", ".uptnl"];
    for ext in all_exts {
        if lower.ends_with(ext) {
            return Some(&path[..path.len() - ext.len()]);
        }
    }
    None
}

/// Load combined asset data: header bytes + optional uexp continuation.
///
/// If the header's exports reference data beyond the header file size,
/// the uexp_data provides the continuation. The combined view allows
/// the property parser to access all export data.
pub fn combined_asset_data(header_data: &[u8], uexp_data: Option<&[u8]>) -> Vec<u8> {
    match uexp_data {
        Some(uexp) if !uexp.is_empty() => {
            let mut combined = Vec::with_capacity(header_data.len() + uexp.len());
            combined.extend_from_slice(header_data);
            combined.extend_from_slice(uexp);
            combined
        }
        _ => header_data.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_header_path() {
        assert!(is_header_path("Content/Maps/Level.umap"));
        assert!(is_header_path("Content/BP_Actor.uasset"));
        assert!(is_header_path("Content/BP_Actor.UASSET"));
        assert!(!is_header_path("Content/BP_Actor.uexp"));
        assert!(!is_header_path("Content/BP_Actor.ubulk"));
    }

    #[test]
    fn test_is_companion_path() {
        assert!(is_companion_path("Content/BP_Actor.uexp"));
        assert!(is_companion_path("Content/BP_Actor.ubulk"));
        assert!(is_companion_path("Content/BP_Actor.uptnl"));
        assert!(!is_companion_path("Content/BP_Actor.uasset"));
    }

    #[test]
    fn test_companion_paths() {
        let companions = companion_paths("Content/BP_Actor.uasset");
        assert_eq!(companions.len(), 3);
        assert!(companions.contains(&"Content/BP_Actor.uexp".to_string()));
        assert!(companions.contains(&"Content/BP_Actor.ubulk".to_string()));
        assert!(companions.contains(&"Content/BP_Actor.uptnl".to_string()));
    }

    #[test]
    fn test_header_for_companion() {
        assert_eq!(
            header_for_companion("Content/BP_Actor.uexp"),
            Some("Content/BP_Actor.uasset".to_string())
        );
        assert_eq!(header_for_companion("Content/BP_Actor.uasset"), None);
    }

    #[test]
    fn test_combined_asset_data() {
        let header = vec![1, 2, 3];
        let uexp = vec![4, 5, 6];
        let combined = combined_asset_data(&header, Some(&uexp));
        assert_eq!(combined, vec![1, 2, 3, 4, 5, 6]);

        let no_uexp = combined_asset_data(&header, None);
        assert_eq!(no_uexp, vec![1, 2, 3]);

        let empty_uexp = combined_asset_data(&header, Some(&[]));
        assert_eq!(empty_uexp, vec![1, 2, 3]);
    }
}
