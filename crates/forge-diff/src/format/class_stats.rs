//! `--class-stats` histogram — per-export-class count delta and K2Node
//! parsing diagnostics.
//!
//! Purpose: when a Blueprint diff shows only a cryptic `~ native data changed`
//! on the generated class and no per-export adds, `--class-stats` reveals
//! whether the export count of a given class (e.g. `K2Node_CallFunction`)
//! actually changed.

use std::collections::BTreeMap;
use forge_unreal::structured::StructuredAsset;

pub fn emit_class_stats(
    old: &StructuredAsset,
    new: &StructuredAsset,
    output: &mut String,
) {
    let mut old_counts: BTreeMap<String, usize> = BTreeMap::new();
    for exp in &old.exports {
        *old_counts.entry(exp.class_name.clone()).or_default() += 1;
    }
    let mut new_counts: BTreeMap<String, usize> = BTreeMap::new();
    for exp in &new.exports {
        *new_counts.entry(exp.class_name.clone()).or_default() += 1;
    }

    let mut classes: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for k in old_counts.keys() {
        classes.insert(k.as_str());
    }
    for k in new_counts.keys() {
        classes.insert(k.as_str());
    }

    let mut lines: Vec<(i64, String)> = Vec::new();
    for class in classes {
        let old_n = *old_counts.get(class).unwrap_or(&0);
        let new_n = *new_counts.get(class).unwrap_or(&0);
        if old_n == new_n {
            continue;
        }
        let delta = new_n as i64 - old_n as i64;
        let sign = if delta > 0 { "+" } else { "" };
        lines.push((
            -delta.abs(),
            format!(
                "  \x1b[36m[class-stats]\x1b[0m {}: {} -> {} ({}{})",
                class, old_n, new_n, sign, delta
            ),
        ));
    }

    if lines.is_empty() {
        output.push_str("  \x1b[2m[class-stats] no export-count differences\x1b[0m\n");
    } else {
        lines.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, line) in lines {
            output.push_str(&line);
            output.push('\n');
        }
    }

    // For K2Node exports, count how many have a parseable NodeGuid vs not —
    // when NodeGuid parsing fails, matching falls back to (outer, object_name)
    // which can alias UE-renumbered nodes.
    let k2_guid_stats = |asset: &StructuredAsset| -> (usize, usize) {
        let mut with = 0;
        let mut without = 0;
        for exp in &asset.exports {
            if !exp.class_name.starts_with("K2Node_") {
                continue;
            }
            let has = exp.properties.as_ref()
                .map(|p| p.iter().any(|tp| tp.name == "NodeGuid"))
                .unwrap_or(false);
            if has { with += 1; } else { without += 1; }
        }
        (with, without)
    };
    let (o_w, o_wo) = k2_guid_stats(old);
    let (n_w, n_wo) = k2_guid_stats(new);
    if o_wo > 0 || n_wo > 0 {
        output.push_str(&format!(
            "  \x1b[33m[class-stats] K2Node NodeGuid parsed: old {}/{} new {}/{} ({} / {} without)\x1b[0m\n",
            o_w, o_w + o_wo, n_w, n_w + n_wo, o_wo, n_wo
        ));
    }

    let k2_prop_stats = |asset: &StructuredAsset| -> (usize, usize, usize) {
        let mut some_nonempty = 0;
        let mut some_empty = 0;
        let mut none = 0;
        for exp in &asset.exports {
            if !exp.class_name.starts_with("K2Node_") { continue; }
            match &exp.properties {
                Some(v) if !v.is_empty() => some_nonempty += 1,
                Some(_) => some_empty += 1,
                None => none += 1,
            }
        }
        (some_nonempty, some_empty, none)
    };
    let (nw_s, nw_e, nw_n) = k2_prop_stats(new);
    output.push_str(&format!(
        "  \x1b[33m[class-stats] K2Node props: {} w/props, {} empty, {} unparsed (new side)\x1b[0m\n",
        nw_s, nw_e, nw_n
    ));
    for exp in &new.exports {
        if !exp.class_name.starts_with("K2Node_") { continue; }
        if let Some(props) = &exp.properties {
            if !props.is_empty() {
                let names: Vec<&str> = props.iter().take(8).map(|p| p.name.as_str()).collect();
                output.push_str(&format!(
                    "  \x1b[33m[class-stats] sample K2Node ({}): props=[{}]\x1b[0m\n",
                    exp.class_name, names.join(", ")
                ));
                break;
            }
        }
    }

    // Show the actual added/removed export names for classes with deltas —
    // diagnoses whether the main diff is filtering these out somewhere downstream.
    let old_names: std::collections::HashSet<&str> =
        old.exports.iter().map(|e| e.object_name.as_str()).collect();
    let new_names: std::collections::HashSet<&str> =
        new.exports.iter().map(|e| e.object_name.as_str()).collect();

    for exp in &new.exports {
        if old_counts.get(&exp.class_name).unwrap_or(&0)
            != new_counts.get(&exp.class_name).unwrap_or(&0)
            && !old_names.contains(exp.object_name.as_str())
        {
            let outer = exp.outer_name.as_deref().unwrap_or("<root>");
            output.push_str(&format!(
                "    \x1b[2m+ {} ({}) outer={}\x1b[0m\n",
                exp.object_name, exp.class_name, outer
            ));
        }
    }
    for exp in &old.exports {
        if old_counts.get(&exp.class_name).unwrap_or(&0)
            != new_counts.get(&exp.class_name).unwrap_or(&0)
            && !new_names.contains(exp.object_name.as_str())
        {
            let outer = exp.outer_name.as_deref().unwrap_or("<root>");
            output.push_str(&format!(
                "    \x1b[2m- {} ({}) outer={}\x1b[0m\n",
                exp.object_name, exp.class_name, outer
            ));
        }
    }
}
