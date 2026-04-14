//! Three-way property-level merge for UE assets.
//!
//! When two branches modify the same `.uasset` file, this module attempts to
//! auto-merge non-conflicting property changes instead of flagging the entire
//! file as a conflict.

use forge_unreal::property::TaggedProperty;
use forge_unreal::structured::{parse_structured_with_uexp, ExportInfo, ImportInfo};
use std::collections::BTreeMap;

/// Result of a three-way asset merge.
pub enum MergeResult {
    /// Both sides changed the file identically — take either version.
    Identical,
    /// No property-level changes detected or parsing failed — cannot merge.
    CannotMerge,
    /// Successfully merged: one side's binary should be used.
    /// `use_ours` indicates which side's raw bytes to keep.
    TakeOurs,
    TakeTheirs,
    /// Both sides made non-conflicting changes.
    /// Contains details of what each side changed, plus reconstruction data.
    AutoMerged {
        ours_changes: Vec<String>,
        theirs_changes: Vec<String>,
        /// Export modifications for binary reconstruction (export_index -> new property bytes).
        /// Empty if reconstruction data couldn't be computed.
        modifications: Vec<MergedExportData>,
    },
    /// Conflicting changes at the property level.
    Conflict(Vec<MergeConflict>),
}

/// Data for reconstructing a merged export's property bytes.
#[derive(Debug)]
pub struct MergedExportData {
    /// Export index in the export table.
    pub export_index: usize,
    /// Serialized tagged property bytes for this export.
    pub property_data: Vec<u8>,
}

/// A specific property-level conflict.
#[derive(Debug)]
pub struct MergeConflict {
    pub export_name: String,
    pub property_path: String,
    pub base_value: Option<String>,
    pub ours_value: Option<String>,
    pub theirs_value: Option<String>,
}

impl std::fmt::Display for MergeConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {}: base={} | ours={} | theirs={}",
            self.export_name,
            self.property_path,
            self.base_value.as_deref().unwrap_or("<none>"),
            self.ours_value.as_deref().unwrap_or("<none>"),
            self.theirs_value.as_deref().unwrap_or("<none>"),
        )
    }
}

/// Attempt a three-way merge of `.uasset` files at the property level.
///
/// `base` is the common ancestor, `ours` is the current branch's version,
/// `theirs` is the incoming branch's version.
///
/// Returns `MergeResult::CannotMerge` if any version fails to parse
/// (the caller should fall back to whole-file conflict).
pub fn merge_assets(base: &[u8], ours: &[u8], theirs: &[u8]) -> MergeResult {
    merge_assets_with_uexp(base, None, ours, None, theirs, None)
}

/// Three-way merge with optional `.uexp` companion data for split assets.
pub fn merge_assets_with_uexp(
    base: &[u8],
    base_uexp: Option<&[u8]>,
    ours: &[u8],
    ours_uexp: Option<&[u8]>,
    theirs: &[u8],
    theirs_uexp: Option<&[u8]>,
) -> MergeResult {
    // Parse all three versions.
    let base_asset = match parse_structured_with_uexp(base, base_uexp) {
        Ok(a) => a,
        Err(_) => return MergeResult::CannotMerge,
    };
    let ours_asset = match parse_structured_with_uexp(ours, ours_uexp) {
        Ok(a) => a,
        Err(_) => return MergeResult::CannotMerge,
    };
    let theirs_asset = match parse_structured_with_uexp(theirs, theirs_uexp) {
        Ok(a) => a,
        Err(_) => return MergeResult::CannotMerge,
    };

    let mut conflicts = Vec::new();
    let mut ours_changes = Vec::new();
    let mut theirs_changes = Vec::new();

    // Merge imports.
    merge_imports(
        &base_asset.imports,
        &ours_asset.imports,
        &theirs_asset.imports,
        &mut conflicts,
        &mut ours_changes,
        &mut theirs_changes,
    );

    // Merge exports and their properties.
    merge_exports(
        &base_asset.exports,
        &ours_asset.exports,
        &theirs_asset.exports,
        &mut conflicts,
        &mut ours_changes,
        &mut theirs_changes,
    );

    if !conflicts.is_empty() {
        return MergeResult::Conflict(conflicts);
    }

    if ours_changes.is_empty() && theirs_changes.is_empty() {
        return MergeResult::Identical;
    }

    // If only one side has changes, we can use that side's binary directly.
    if ours_changes.is_empty() {
        return MergeResult::TakeTheirs;
    }
    if theirs_changes.is_empty() {
        return MergeResult::TakeOurs;
    }

    // Both sides have non-conflicting changes.
    // Build merged property data for reconstruction.
    // Strategy: start from "ours" binary, apply "theirs" non-conflicting changes.
    // For each export that "theirs" modified, we need to produce the merged property list.
    let modifications = build_merged_exports(
        &base_asset,
        &ours_asset,
        &theirs_asset,
    );

    MergeResult::AutoMerged {
        ours_changes,
        theirs_changes,
        modifications,
    }
}

/// Build merged export data by combining ours + theirs non-conflicting changes.
///
/// For each export where "theirs" made property changes, produces the merged
/// property list by starting from "ours" properties and applying "theirs" changes.
fn build_merged_exports(
    base: &forge_unreal::structured::StructuredAsset,
    ours: &forge_unreal::structured::StructuredAsset,
    theirs: &forge_unreal::structured::StructuredAsset,
) -> Vec<MergedExportData> {
    use forge_unreal::property::serialize_tagged_properties;
    use std::collections::BTreeMap;

    let base_map: BTreeMap<&str, &ExportInfo> =
        base.exports.iter().map(|e| (e.object_name.as_str(), e)).collect();
    let ours_map: BTreeMap<&str, (usize, &ExportInfo)> =
        ours.exports.iter().enumerate().map(|(i, e)| (e.object_name.as_str(), (i, e))).collect();
    let theirs_map: BTreeMap<&str, &ExportInfo> =
        theirs.exports.iter().map(|e| (e.object_name.as_str(), e)).collect();

    let mut modifications = Vec::new();

    for (name, base_exp) in &base_map {
        let ours_entry = match ours_map.get(name) {
            Some(e) => e,
            None => continue,
        };
        let theirs_exp = match theirs_map.get(name) {
            Some(e) => e,
            None => continue,
        };

        let (ours_idx, ours_exp) = ours_entry;

        // We only need to produce reconstruction data when theirs changed this export.
        // (If only ours changed, ours binary already has the right data.)
        let base_props = match &base_exp.properties {
            Some(p) => p,
            None => continue,
        };
        let ours_props = match &ours_exp.properties {
            Some(p) => p,
            None => continue,
        };
        let theirs_props = match &theirs_exp.properties {
            Some(p) => p,
            None => continue,
        };

        // Check if theirs actually changed from base.
        if theirs_props == base_props {
            continue; // No theirs changes — ours binary is already correct.
        }
        // Check if ours also changed (both changed = need merged version).
        // If only theirs changed, we need to apply theirs to base.
        // If both changed non-conflictingly, we need to merge.

        // Build merged property list: start from ours, apply theirs' additions/changes.
        let merged = merge_property_lists(base_props, ours_props, theirs_props);

        let mut names = ours.names.clone();
        let serialized = serialize_tagged_properties(&merged, &mut names);

        modifications.push(MergedExportData {
            export_index: *ours_idx,
            property_data: serialized,
        });
    }

    modifications
}

/// Merge three property lists: base, ours, theirs -> merged.
/// Assumes no conflicts (caller already verified this).
fn merge_property_lists(
    base: &[TaggedProperty],
    ours: &[TaggedProperty],
    theirs: &[TaggedProperty],
) -> Vec<TaggedProperty> {
    use std::collections::BTreeMap;

    let base_map: BTreeMap<(&str, u32), &TaggedProperty> =
        base.iter().map(|p| ((p.name.as_str(), p.array_index), p)).collect();
    let ours_map: BTreeMap<(&str, u32), &TaggedProperty> =
        ours.iter().map(|p| ((p.name.as_str(), p.array_index), p)).collect();
    let theirs_map: BTreeMap<(&str, u32), &TaggedProperty> =
        theirs.iter().map(|p| ((p.name.as_str(), p.array_index), p)).collect();

    let mut result = Vec::new();

    // Start with ours as the base, apply theirs' changes.
    // Include all properties from ours.
    for ((name, idx), ours_prop) in &ours_map {
        let base_val = base_map.get(&(*name, *idx));
        let theirs_val = theirs_map.get(&(*name, *idx));

        match (base_val, theirs_val) {
            // Theirs changed this property (ours didn't, or both same).
            (Some(b), Some(t)) if *b != *t && *ours_prop == *b => {
                result.push((*t).clone());
            }
            // Theirs deleted this property.
            (Some(b), None) if *ours_prop == *b => {
                // Skip — theirs deleted it.
            }
            // Default: keep ours.
            _ => {
                result.push((*ours_prop).clone());
            }
        }
    }

    // Add properties that theirs added (not in base, not in ours).
    for ((name, idx), theirs_prop) in &theirs_map {
        if !ours_map.contains_key(&(*name, *idx)) && !base_map.contains_key(&(*name, *idx)) {
            result.push((*theirs_prop).clone());
        }
    }

    result
}

fn merge_imports(
    base: &[ImportInfo],
    ours: &[ImportInfo],
    theirs: &[ImportInfo],
    _conflicts: &mut Vec<MergeConflict>,
    ours_changes: &mut Vec<String>,
    theirs_changes: &mut Vec<String>,
) {
    let base_set: BTreeMap<&str, &ImportInfo> =
        base.iter().map(|i| (i.object_name.as_str(), i)).collect();
    let ours_set: BTreeMap<&str, &ImportInfo> =
        ours.iter().map(|i| (i.object_name.as_str(), i)).collect();
    let theirs_set: BTreeMap<&str, &ImportInfo> =
        theirs.iter().map(|i| (i.object_name.as_str(), i)).collect();

    // Imports added by ours only.
    for name in ours_set.keys() {
        if !base_set.contains_key(name) && !theirs_set.contains_key(name) {
            ours_changes.push(format!("+ import: {}", name));
        }
    }
    // Imports added by theirs only.
    for name in theirs_set.keys() {
        if !base_set.contains_key(name) && !ours_set.contains_key(name) {
            theirs_changes.push(format!("+ import: {}", name));
        }
    }
    // Both added same import — fine (convergent).
    // Both added different imports — also fine (union).
    // Imports removed by one but not the other — only conflict if other modified it.
    for name in base_set.keys() {
        let in_ours = ours_set.contains_key(name);
        let in_theirs = theirs_set.contains_key(name);
        if !in_ours && in_theirs {
            ours_changes.push(format!("- import: {}", name));
        }
        if in_ours && !in_theirs {
            theirs_changes.push(format!("- import: {}", name));
        }
    }
}

fn merge_exports(
    base: &[ExportInfo],
    ours: &[ExportInfo],
    theirs: &[ExportInfo],
    conflicts: &mut Vec<MergeConflict>,
    ours_changes: &mut Vec<String>,
    theirs_changes: &mut Vec<String>,
) {
    let base_map: BTreeMap<&str, &ExportInfo> =
        base.iter().map(|e| (e.object_name.as_str(), e)).collect();
    let ours_map: BTreeMap<&str, &ExportInfo> =
        ours.iter().map(|e| (e.object_name.as_str(), e)).collect();
    let theirs_map: BTreeMap<&str, &ExportInfo> =
        theirs.iter().map(|e| (e.object_name.as_str(), e)).collect();

    // Exports added/removed.
    for name in ours_map.keys() {
        if !base_map.contains_key(name) && !theirs_map.contains_key(name) {
            ours_changes.push(format!("+ export: {}", name));
        }
    }
    for name in theirs_map.keys() {
        if !base_map.contains_key(name) && !ours_map.contains_key(name) {
            theirs_changes.push(format!("+ export: {}", name));
        }
    }
    // Both added same export — potential conflict if properties differ.
    for name in ours_map.keys() {
        if !base_map.contains_key(name) && theirs_map.contains_key(name) {
            let ours_exp = ours_map[name];
            let theirs_exp = theirs_map[name];
            if let (Some(op), Some(tp)) = (&ours_exp.properties, &theirs_exp.properties) {
                if op != tp {
                    conflicts.push(MergeConflict {
                        export_name: name.to_string(),
                        property_path: "<new export>".to_string(),
                        base_value: None,
                        ours_value: Some(format!("{} properties", op.len())),
                        theirs_value: Some(format!("{} properties", tp.len())),
                    });
                }
            }
        }
    }

    // Merge properties within matched exports.
    for (name, base_exp) in &base_map {
        let ours_exp = ours_map.get(name);
        let theirs_exp = theirs_map.get(name);

        match (ours_exp, theirs_exp) {
            (Some(o), Some(t)) => {
                // Both have this export — merge properties.
                if let (Some(bp), Some(op), Some(tp)) =
                    (&base_exp.properties, &o.properties, &t.properties)
                {
                    merge_properties(
                        name,
                        bp,
                        op,
                        tp,
                        conflicts,
                        ours_changes,
                        theirs_changes,
                    );
                }
                // Also check trailing data changes.
                if o.trailing_data_size != base_exp.trailing_data_size
                    && t.trailing_data_size != base_exp.trailing_data_size
                    && o.trailing_data_size != t.trailing_data_size
                {
                    conflicts.push(MergeConflict {
                        export_name: name.to_string(),
                        property_path: "<native data>".to_string(),
                        base_value: Some(format!("{} bytes", base_exp.trailing_data_size)),
                        ours_value: Some(format!("{} bytes", o.trailing_data_size)),
                        theirs_value: Some(format!("{} bytes", t.trailing_data_size)),
                    });
                }
            }
            (None, Some(_)) => {
                // Deleted in ours, present in theirs.
                ours_changes.push(format!("- export: {}", name));
            }
            (Some(_), None) => {
                // Present in ours, deleted in theirs.
                theirs_changes.push(format!("- export: {}", name));
            }
            (None, None) => {
                // Both deleted — fine.
            }
        }
    }
}

fn merge_properties(
    export_name: &str,
    base_props: &[TaggedProperty],
    ours_props: &[TaggedProperty],
    theirs_props: &[TaggedProperty],
    conflicts: &mut Vec<MergeConflict>,
    ours_changes: &mut Vec<String>,
    theirs_changes: &mut Vec<String>,
) {
    let base_map: BTreeMap<(&str, u32), &TaggedProperty> = base_props
        .iter()
        .map(|p| ((p.name.as_str(), p.array_index), p))
        .collect();
    let ours_map: BTreeMap<(&str, u32), &TaggedProperty> = ours_props
        .iter()
        .map(|p| ((p.name.as_str(), p.array_index), p))
        .collect();
    let theirs_map: BTreeMap<(&str, u32), &TaggedProperty> = theirs_props
        .iter()
        .map(|p| ((p.name.as_str(), p.array_index), p))
        .collect();

    // Collect all property keys.
    let mut all_keys: Vec<(&str, u32)> = Vec::new();
    for k in base_map.keys().chain(ours_map.keys()).chain(theirs_map.keys()) {
        if !all_keys.contains(k) {
            all_keys.push(*k);
        }
    }

    for key in &all_keys {
        let (name, idx) = key;
        let prop_path = if *idx > 0 {
            format!("{}[{}]", name, idx)
        } else {
            name.to_string()
        };

        let base_val = base_map.get(key).map(|p| &p.value);
        let ours_val = ours_map.get(key).map(|p| &p.value);
        let theirs_val = theirs_map.get(key).map(|p| &p.value);

        match (base_val, ours_val, theirs_val) {
            // All three same — no change.
            (Some(b), Some(o), Some(t)) if o == b && t == b => {}
            // Only ours changed.
            (Some(b), Some(o), Some(t)) if t == b && o != b => {
                ours_changes.push(format!("[{}] ~ {}", export_name, prop_path));
            }
            // Only theirs changed.
            (Some(b), Some(o), Some(t)) if o == b && t != b => {
                theirs_changes.push(format!("[{}] ~ {}", export_name, prop_path));
            }
            // Both changed to same value — convergent.
            (Some(_b), Some(o), Some(t)) if o == t => {}
            // Both changed differently — conflict.
            (Some(b), Some(o), Some(t)) => {
                conflicts.push(MergeConflict {
                    export_name: export_name.to_string(),
                    property_path: prop_path,
                    base_value: Some(format!("{}", b)),
                    ours_value: Some(format!("{}", o)),
                    theirs_value: Some(format!("{}", t)),
                });
            }
            // Property added only by ours.
            (None, Some(_o), None) => {
                ours_changes.push(format!("[{}] + {}", export_name, prop_path));
            }
            // Property added only by theirs.
            (None, None, Some(_t)) => {
                theirs_changes.push(format!("[{}] + {}", export_name, prop_path));
            }
            // Both added same value — convergent.
            (None, Some(o), Some(t)) if o == t => {}
            // Both added different values — conflict.
            (None, Some(o), Some(t)) => {
                conflicts.push(MergeConflict {
                    export_name: export_name.to_string(),
                    property_path: prop_path,
                    base_value: None,
                    ours_value: Some(format!("{}", o)),
                    theirs_value: Some(format!("{}", t)),
                });
            }
            // Deleted by ours, unchanged in theirs.
            (Some(b), None, Some(t)) if t == b => {
                ours_changes.push(format!("[{}] - {}", export_name, prop_path));
            }
            // Deleted by theirs, unchanged in ours.
            (Some(b), Some(o), None) if o == b => {
                theirs_changes.push(format!("[{}] - {}", export_name, prop_path));
            }
            // Deleted by one, modified by other — conflict.
            (Some(b), None, Some(t)) => {
                conflicts.push(MergeConflict {
                    export_name: export_name.to_string(),
                    property_path: prop_path,
                    base_value: Some(format!("{}", b)),
                    ours_value: None,
                    theirs_value: Some(format!("{}", t)),
                });
            }
            (Some(b), Some(o), None) => {
                conflicts.push(MergeConflict {
                    export_name: export_name.to_string(),
                    property_path: prop_path,
                    base_value: Some(format!("{}", b)),
                    ours_value: Some(format!("{}", o)),
                    theirs_value: None,
                });
            }
            // Both deleted — fine.
            (Some(_), None, None) => {}
            // No entry anywhere.
            (None, None, None) => {}
        }
    }
}
