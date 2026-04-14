//! UserDefinedEnum diff — enumerator adds/removes via name-table inspection.

use std::collections::BTreeMap;
use forge_unreal::structured::ExportInfo;

use super::{AssetDiffHandler, DiffContext};
use crate::change::AssetChange;

pub struct EnumHandler;

impl AssetDiffHandler for EnumHandler {
    fn name(&self) -> &'static str { "enum_values" }

    fn diff_top_level(&self, ctx: &DiffContext<'_>, sink: &mut Vec<AssetChange>) {
        diff_enum_values(
            &ctx.old.asset.exports,
            ctx.old.names(),
            ctx.old.raw_data,
            &ctx.new.asset.exports,
            ctx.new.names(),
            ctx.new.raw_data,
            sink,
        );
    }
}

/// Diff UserDefinedEnum enumerators by comparing name table entries.
///
/// UE enum values are stored as FName entries in the name table with the pattern
/// `EnumName::EnumeratorName`. By comparing which enum-prefixed names exist in
/// the old vs new name tables, we can detect added/removed enumerators.
fn diff_enum_values(
    old_exports: &[ExportInfo],
    old_names: &[String],
    old_data: Option<&[u8]>,
    new_exports: &[ExportInfo],
    new_names: &[String],
    new_data: Option<&[u8]>,
    changes: &mut Vec<AssetChange>,
) {
    let old_map: BTreeMap<&str, &ExportInfo> =
        old_exports.iter().map(|e| (e.object_name.as_str(), e)).collect();
    let new_map: BTreeMap<&str, &ExportInfo> =
        new_exports.iter().map(|e| (e.object_name.as_str(), e)).collect();

    for (name, new_exp) in &new_map {
        if !is_enum_export(&new_exp.class_name) {
            continue;
        }

        let prefix = format!("{}::", name);

        let new_values: Vec<&str> = new_names.iter()
            .filter(|n| n.starts_with(&prefix))
            .map(|n| &n[prefix.len()..])
            .collect();

        let old_values: Vec<&str> = if old_map.contains_key(name) {
            old_names.iter()
                .filter(|n| n.starts_with(&prefix))
                .map(|n| &n[prefix.len()..])
                .collect()
        } else {
            Vec::new()
        };

        if new_values == old_values {
            continue;
        }

        let new_display = new_data
            .map(|d| build_enum_display_map_from_data(name, &new_values, new_names, d))
            .unwrap_or_default();
        let old_display = old_data
            .map(|d| build_enum_display_map_from_data(name, &old_values, old_names, d))
            .unwrap_or_default();

        for val in &new_values {
            if !old_values.contains(val) && !val.ends_with("_MAX") {
                let display = new_display.get(*val).cloned()
                    .unwrap_or_else(|| val.to_string());
                changes.push(AssetChange::EnumValueAdded {
                    export_name: name.to_string(),
                    value_name: display,
                    display_name: None,
                });
            }
        }

        for val in &old_values {
            if !new_values.contains(val) && !val.ends_with("_MAX") {
                let display = old_display.get(*val).cloned()
                    .unwrap_or_else(|| val.to_string());
                changes.push(AssetChange::EnumValueRemoved {
                    export_name: name.to_string(),
                    value_name: display,
                });
            }
        }
    }
}

fn is_enum_export(class_name: &str) -> bool {
    class_name == "UserDefinedEnum" || class_name == "Enum"
}

/// Build a map from enumerator internal name to display name by scanning raw data.
///
/// Display names are stored as FText values in the DisplayNameMap MapProperty.
/// They appear as inline FStrings in the raw data, in the same order as the
/// enumerators. We collect all display-name-like FStrings from the export data
/// and match them to enumerators by order.
fn build_enum_display_map_from_data(
    _enum_name: &str,
    values: &[&str],
    _names: &[String],
    data: &[u8],
) -> BTreeMap<String, String> {
    let mut display_map = BTreeMap::new();

    const EXCLUDE: &[&str] = &[
        "None", "Class", "Package", "MapProperty", "NameProperty", "TextProperty",
        "StrProperty", "IntProperty", "BoolProperty", "EnumProperty", "StructProperty",
        "ArrayProperty", "UInt32Property", "ObjectProperty", "ByteProperty",
        "UserDefinedEnum", "BlueprintType", "PackageLocalizationNamespace",
        "UniqueNameIndex", "true", "false", "EnumDescription", "DisplayNameMap",
    ];

    let non_max_values: Vec<&&str> = values.iter()
        .filter(|v| !v.ends_with("_MAX"))
        .collect();

    let mut display_names: Vec<String> = Vec::new();

    let mut off = 0usize;
    while off + 4 < data.len() {
        let Ok(lb) = data[off..off + 4].try_into() else { off += 1; continue };
        let length = i32::from_le_bytes(lb);

        if length >= 3 && length <= 50 {
            let str_start = off + 4;
            let str_end = str_start + length as usize;
            if str_end <= data.len() {
                let bytes = &data[str_start..str_end - 1];
                if !bytes.is_empty()
                    && data[str_end - 1] == 0
                    && bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ')
                {
                    let text = String::from_utf8_lossy(bytes).to_string();
                    let looks_like_guid = (text.len() == 32
                        && text.chars().all(|c| c.is_ascii_hexdigit()))
                        || (text.starts_with('[') && text.ends_with(']')
                            && text.len() > 20
                            && text[1..text.len()-1].chars().all(|c| c.is_ascii_hexdigit()));
                    if !text.starts_with('/')
                        && !text.starts_with('+')
                        && !text.contains("::")
                        && !text.contains('.')
                        && !text.starts_with("NewEnumerator")
                        && !text.starts_with("E_")
                        && !EXCLUDE.contains(&text.as_str())
                        && !text.contains("_MAX")
                        && !looks_like_guid
                    {
                        if !display_names.contains(&text) {
                            display_names.push(text);
                        }
                    }
                    off = str_end;
                    continue;
                }
            }
        }
        off += 1;
    }

    for (i, val) in non_max_values.iter().enumerate() {
        if i < display_names.len() {
            display_map.insert(val.to_string(), display_names[i].clone());
        }
    }

    display_map
}
