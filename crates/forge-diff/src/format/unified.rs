//! Structured asset diff → hierarchical colored output.
//!
//! Parses two uasset byte blobs, runs the diff engine, then organises the
//! resulting [`AssetChange`] list into a tree grouped by export outer
//! (parent), with K2Node label decoration and UE auto-renumber collapsing.

use std::collections::{BTreeMap, HashMap, HashSet};

use forge_unreal::structured::parse_structured_with_uexp;

use super::class_stats::emit_class_stats;
use super::renumber::collapse_renumber_pairs;
use crate::change::AssetChange;
use crate::engine::diff_assets_with_data;
use crate::label::extract_k2node_label;

/// Attempt a structured diff with optional .uexp companion data.
///
/// Returns `None` if parsing fails or no semantic changes are detected —
/// caller falls back to the opaque `Binary files … differ` line.
pub fn try_structured_asset_diff_with_uexp(
    path: &str,
    old_data: &[u8],
    old_uexp: Option<&[u8]>,
    new_data: &[u8],
    new_uexp: Option<&[u8]>,
    class_stats: bool,
) -> Option<String> {
    let old_asset = parse_structured_with_uexp(old_data, old_uexp).ok()?;
    let new_asset = parse_structured_with_uexp(new_data, new_uexp).ok()?;

    let changes = diff_assets_with_data(
        &old_asset, Some(old_data),
        &new_asset, Some(new_data),
    );

    if changes.is_empty() {
        return None;
    }

    // Build lookup maps for outer (parent) names.
    let new_outer: HashMap<String, Option<String>> = new_asset.exports.iter()
        .map(|e| (e.object_name.clone(), e.outer_name.clone()))
        .collect();
    let old_outer: HashMap<String, Option<String>> = old_asset.exports.iter()
        .map(|e| (e.object_name.clone(), e.outer_name.clone()))
        .collect();
    let new_import_outer: HashMap<String, Option<String>> = new_asset.imports.iter()
        .map(|i| (i.object_name.clone(), i.outer_name.clone()))
        .collect();
    let old_import_outer: HashMap<String, Option<String>> = old_asset.imports.iter()
        .map(|i| (i.object_name.clone(), i.outer_name.clone()))
        .collect();

    let mut output = String::new();
    output.push_str(&format!(
        "\x1b[1mdiff --forge a/{} b/{}\x1b[0m\n",
        path, path
    ));
    output.push_str(&format!(
        "  \x1b[36m[asset]\x1b[0m Engine: {} | Exports: {} | Imports: {}\n",
        new_asset.engine_version,
        new_asset.exports.len(),
        new_asset.imports.len()
    ));

    if !new_asset.parse_warnings.is_empty() {
        for w in &new_asset.parse_warnings {
            output.push_str(&format!("  \x1b[33mwarning: {}\x1b[0m\n", w));
        }
    }

    if class_stats {
        emit_class_stats(&old_asset, &new_asset, &mut output);
    }

    // Separate changes into categories for hierarchical display.
    let mut import_adds: Vec<&forge_unreal::structured::ImportInfo> = Vec::new();
    let mut import_removes: Vec<&forge_unreal::structured::ImportInfo> = Vec::new();
    let mut export_adds: Vec<(String, String)> = Vec::new();
    let mut export_removes: Vec<(String, String)> = Vec::new();
    let mut property_changes: Vec<&AssetChange> = Vec::new();

    for change in &changes {
        match change {
            AssetChange::ImportAdded(imp) => import_adds.push(imp),
            AssetChange::ImportRemoved(imp) => import_removes.push(imp),
            AssetChange::ExportAdded { name, class } => {
                export_adds.push((name.clone(), class.clone()));
            }
            AssetChange::ExportRemoved { name, class } => {
                export_removes.push((name.clone(), class.clone()));
            }
            _ => property_changes.push(change),
        }
    }

    let renumbered_pairs = collapse_renumber_pairs(&mut export_adds, &mut export_removes);
    if renumbered_pairs > 0 {
        output.push_str(&format!(
            "  \x1b[2m(collapsed {} UE auto-renumber pair{})\x1b[0m\n",
            renumbered_pairs,
            if renumbered_pairs == 1 { "" } else { "s" }
        ));
    }

    // --- Imports: group by package, combine adds/removes ---
    {
        let mut import_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for imp in &import_removes {
            let outer = old_import_outer.get(&imp.object_name)
                .and_then(|o| o.clone()).unwrap_or_default();
            import_groups.entry(outer).or_default().push(
                format!("  \x1b[31m- {} ({})\x1b[0m", imp.object_name, imp.class_name)
            );
        }
        for imp in &import_adds {
            let outer = new_import_outer.get(&imp.object_name)
                .and_then(|o| o.clone()).unwrap_or_default();
            import_groups.entry(outer).or_default().push(
                format!("  \x1b[32m+ {} ({})\x1b[0m", imp.object_name, imp.class_name)
            );
        }

        for (outer, lines) in &import_groups {
            if outer.is_empty() || lines.len() == 1 {
                for line in lines {
                    output.push_str(&format!("  import:{}\n", line.trim_start()));
                }
            } else {
                output.push_str(&format!("  \x1b[36m[import: {}]\x1b[0m\n", outer));
                for line in lines {
                    output.push_str(&format!("  {}\n", line));
                }
            }
        }
    }

    // --- Exports + property changes: unified by export name ---
    let mut export_changes: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // K2Node label decoration: `K2Node_CallFunction_9` alone tells a reviewer
    // nothing; `PrintString [K2Node_CallFunction_9]` shows which call changed.
    let old_labels: HashMap<&str, String> = old_asset.exports.iter()
        .filter_map(|e| {
            let label = extract_k2node_label(e, Some(old_data), &old_asset.names)?;
            Some((e.object_name.as_str(), label))
        })
        .collect();
    let new_labels: HashMap<&str, String> = new_asset.exports.iter()
        .filter_map(|e| {
            let label = extract_k2node_label(e, Some(new_data), &new_asset.names)?;
            Some((e.object_name.as_str(), label))
        })
        .collect();

    let decorate = |name: &str, is_new: bool| -> String {
        let label = if is_new {
            new_labels.get(name).cloned()
        } else {
            old_labels.get(name).cloned()
        };
        match label {
            Some(l) => format!("{} [{}]", l, name),
            None => name.to_string(),
        }
    };

    for (name, class) in &export_removes {
        export_changes.entry(name.clone()).or_default().push(
            format!("\x1b[31m- {} ({})\x1b[0m", decorate(name, false), class)
        );
    }
    for (name, class) in &export_adds {
        export_changes.entry(name.clone()).or_default().push(
            format!("\x1b[32m+ {} ({})\x1b[0m", decorate(name, true), class)
        );
    }
    for change in &property_changes {
        match change {
            AssetChange::PropertyChanged {
                export_name, property_path, old_value, new_value,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[33m~ {}\x1b[0m: {} \x1b[33m->\x1b[0m {}", property_path, old_value, new_value)
                );
            }
            AssetChange::PropertyAdded {
                export_name, property_name, value,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[32m+ {}\x1b[0m: {}", property_name, value)
                );
            }
            AssetChange::PropertyRemoved {
                export_name, property_name, value,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[31m- {}\x1b[0m: {}", property_name, value)
                );
            }
            AssetChange::ExportDataChanged {
                export_name, description,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[33m~ {}\x1b[0m", description)
                );
            }
            AssetChange::FieldAdded {
                export_name, field,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[32m+ variable: {}\x1b[0m", field)
                );
            }
            AssetChange::FieldRemoved {
                export_name, field,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[31m- variable: {}\x1b[0m", field)
                );
            }
            AssetChange::EnumValueAdded {
                export_name, value_name, display_name,
            } => {
                let label = if let Some(dn) = display_name {
                    format!("{} ({})", value_name, dn)
                } else {
                    value_name.clone()
                };
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[32m+ enum: {}\x1b[0m", label)
                );
            }
            AssetChange::EnumValueRemoved {
                export_name, value_name,
            } => {
                export_changes.entry(export_name.clone()).or_default().push(
                    format!("\x1b[31m- enum: {}\x1b[0m", value_name)
                );
            }
            _ => {}
        }
    }

    // Now build the tree display.
    let mut all_outer: HashMap<String, Option<String>> = old_outer;
    for (k, v) in &new_outer {
        all_outer.entry(k.clone()).or_insert_with(|| v.clone());
    }

    let changed_export_set: HashSet<&str> = export_adds.iter()
        .map(|(n, _)| n.as_str())
        .chain(export_removes.iter().map(|(n, _)| n.as_str()))
        .collect();

    let mut tree_children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut tree_roots: Vec<String> = Vec::new();

    for name in changed_export_set.iter() {
        let parent = all_outer.get(*name)
            .and_then(|o| o.as_deref())
            .unwrap_or("");
        if parent.is_empty() || !changed_export_set.contains(parent) {
            tree_roots.push(name.to_string());
        } else {
            tree_children.entry(parent.to_string())
                .or_default()
                .push(name.to_string());
        }
    }

    let mut root_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in &tree_roots {
        let parent = all_outer.get(name.as_str())
            .and_then(|o| o.clone())
            .unwrap_or_default();
        root_groups.entry(parent).or_default().push(name.clone());
    }

    let mut prop_only: Vec<String> = Vec::new();
    for name in export_changes.keys() {
        if !changed_export_set.contains(name.as_str()) {
            prop_only.push(name.clone());
        }
    }

    let mut displayed: HashSet<String> = HashSet::new();

    for (context, roots) in &root_groups {
        if !context.is_empty() {
            output.push_str(&format!("  \x1b[36m[{}]\x1b[0m", context));
            if let Some(lines) = export_changes.get(context.as_str()) {
                if !changed_export_set.contains(context.as_str()) {
                    output.push('\n');
                    for line in lines {
                        output.push_str(&format!("    {}\n", line));
                    }
                    displayed.insert(context.clone());
                } else {
                    output.push('\n');
                }
            } else {
                output.push('\n');
            }
        }
        for root_name in roots {
            write_unified_tree_node(
                &mut output, root_name, &export_changes, &tree_children, &all_outer, 2,
            );
            displayed.insert(root_name.clone());
        }
    }

    let mut prop_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in &prop_only {
        if displayed.contains(name) {
            continue;
        }
        let parent = all_outer.get(name.as_str())
            .and_then(|o| o.clone())
            .unwrap_or_default();
        prop_groups.entry(parent).or_default().push(name.clone());
    }

    for (context, names) in &prop_groups {
        let show_context = !context.is_empty();
        if show_context && names.len() > 1 {
            output.push_str(&format!("  \x1b[36m[{}]\x1b[0m\n", context));
            for name in names {
                if let Some(lines) = export_changes.get(name) {
                    for line in lines {
                        output.push_str(&format!("    \x1b[36m[{}]\x1b[0m {}\n", name, line));
                    }
                }
            }
        } else {
            for name in names {
                if let Some(lines) = export_changes.get(name) {
                    let label = if show_context { context.as_str() } else { name.as_str() };
                    if lines.len() == 1 {
                        output.push_str(&format!("  \x1b[36m[{}]\x1b[0m {}\n", label, lines[0]));
                    } else {
                        output.push_str(&format!("  \x1b[36m[{}]\x1b[0m\n", label));
                        for line in lines {
                            output.push_str(&format!("    {}\n", line));
                        }
                    }
                }
            }
        }
    }

    Some(output)
}

fn write_unified_tree_node(
    output: &mut String,
    name: &str,
    export_changes: &BTreeMap<String, Vec<String>>,
    tree_children: &BTreeMap<String, Vec<String>>,
    all_outer: &HashMap<String, Option<String>>,
    depth: usize,
) {
    let indent = "  ".repeat(depth);

    if let Some(lines) = export_changes.get(name) {
        for line in lines {
            output.push_str(&format!("{}{}\n", indent, line));
        }
    }

    if let Some(children) = tree_children.get(name) {
        let mut sorted = children.clone();
        sorted.sort();
        for child in &sorted {
            write_unified_tree_node(output, child, export_changes, tree_children, all_outer, depth + 1);
        }
    }
}
