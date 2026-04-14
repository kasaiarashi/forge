//! Path predicates and binary detection helpers shared by CLI and formatters.

/// Null-byte heuristic for binary detection (first 8 KiB).
pub fn is_binary(data: &[u8]) -> bool {
    data.iter().take(8192).any(|&b| b == 0)
}

/// UE asset header (.uasset / .umap) path predicate.
pub fn is_uasset_path(path: &str) -> bool {
    forge_core::asset_group::is_header_path(path)
}

/// UE companion path predicate (.uexp / .ubulk / .uptnl).
pub fn is_ue_companion_path(path: &str) -> bool {
    forge_core::asset_group::is_companion_path(path)
}
