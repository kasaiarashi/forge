//! Pattern-based scanner for `SerializeProperties()` output inside class /
//! struct exports. Names variables added or removed across versions so diffs
//! can report `+ MyVar (BoolProperty)` instead of "trailing data changed".

use serde::{Deserialize, Serialize};

/// One property declaration recovered from an FField stream.
///
/// Only `field_type`, `field_name` and `array_dim` are filled in by the
/// scanner today — the rest are placeholders that downstream callers fill in
/// where they have richer schema information.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldDefinition {
    pub field_type: String,
    pub field_name: String,
    pub array_dim: i32,
    pub property_flags: u64,
    pub struct_type: Option<String>,
    pub inner_type: Option<String>,
    pub key_type: Option<String>,
    pub value_type: Option<String>,
}

impl std::fmt::Display for FieldDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let descriptor = if let Some(st) = &self.struct_type {
            format!("{}<{}>", self.field_type, st)
        } else if let Some(it) = &self.inner_type {
            format!("{}<{}>", self.field_type, it)
        } else if let (Some(kt), Some(vt)) = (&self.key_type, &self.value_type) {
            format!("{}<{}, {}>", self.field_type, kt, vt)
        } else {
            self.field_type.clone()
        };

        if self.array_dim > 1 {
            write!(f, "{} {}[{}]", descriptor, self.field_name, self.array_dim)
        } else {
            write!(f, "{} {}", descriptor, self.field_name)
        }
    }
}

/// FField class names emitted by UE 4.25+'s unified FField hierarchy. Used as
/// "valid type FName" anchors when searching the byte stream.
const KNOWN_FIELD_TYPES: &[&str] = &[
    "BoolProperty",
    "ByteProperty",
    "Int8Property",
    "Int16Property",
    "IntProperty",
    "Int64Property",
    "UInt16Property",
    "UInt32Property",
    "UInt64Property",
    "FloatProperty",
    "DoubleProperty",
    "StrProperty",
    "NameProperty",
    "TextProperty",
    "ObjectProperty",
    "ClassProperty",
    "SoftObjectProperty",
    "SoftClassProperty",
    "WeakObjectProperty",
    "LazyObjectProperty",
    "InterfaceProperty",
    "StructProperty",
    "ArrayProperty",
    "MapProperty",
    "SetProperty",
    "EnumProperty",
    "DelegateProperty",
    "MulticastDelegateProperty",
    "MulticastInlineDelegateProperty",
    "MulticastSparseDelegateProperty",
    "FieldPathProperty",
    "OptionalProperty",
];

/// The smallest plausible FField record on disk: two FNames + flags + array
/// dim + element size + property flags + rep info — 53 bytes. Used as the
/// per-iteration skip to keep the scanner from re-discovering the field it
/// just parsed.
const MIN_FIELD_SKIP: usize = 53;

/// Try to extract field definitions from `export_data`. Returns the largest
/// successful set (Blueprints can have multiple — inherited from parent C++
/// class plus the BP variables themselves). Returns `None` for non-class /
/// non-struct exports.
pub fn parse_field_definitions(
    export_data: &[u8],
    names: &[String],
    class_name: &str,
) -> Option<Vec<FieldDefinition>> {
    if !is_field_bearing_class(class_name) {
        return None;
    }

    let candidates = scan_all_definitions(export_data, names);
    candidates.into_iter().max_by_key(|set| set.len())
}

fn is_field_bearing_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "Class"
            | "ScriptStruct"
            | "BlueprintGeneratedClass"
            | "WidgetBlueprintGeneratedClass"
            | "AnimBlueprintGeneratedClass"
    ) || class_name.ends_with("GeneratedClass")
}

/// Walk every offset and try to interpret it as `i32 PropertyCount` followed
/// by an FField list. Each match that yields ≥1 field is collected.
fn scan_all_definitions(data: &[u8], names: &[String]) -> Vec<Vec<FieldDefinition>> {
    let mut hits: Vec<Vec<FieldDefinition>> = Vec::new();
    if data.len() < 8 {
        return hits;
    }

    let limit = data.len().saturating_sub(12);
    for off in 0..limit {
        let count = match data[off..off + 4].try_into() {
            Ok(b) => i32::from_le_bytes(b),
            Err(_) => continue,
        };
        if !(1..=500).contains(&count) {
            continue;
        }

        let raw_idx = match data[off + 4..off + 8].try_into() {
            Ok(b) => u32::from_le_bytes(b),
            Err(_) => continue,
        };
        let idx = raw_idx as usize;
        let Some(first_type) = names.get(idx) else {
            continue;
        };
        if !KNOWN_FIELD_TYPES.contains(&first_type.as_str()) {
            continue;
        }

        if let Some(parsed) = try_parse_at(data, off, count as usize, names) {
            if !parsed.is_empty() {
                hits.push(parsed);
            }
        }
    }
    hits
}

/// Walk forward through the data, picking up `count` FField type FNames in
/// sequence. The exact byte layout per field varies by UE version, so we use
/// a "find next known type" heuristic + `MIN_FIELD_SKIP` to advance.
fn try_parse_at(
    data: &[u8],
    offset: usize,
    count: usize,
    names: &[String],
) -> Option<Vec<FieldDefinition>> {
    let mut cursor = offset + 4; // step past PropertyCount
    let mut raw = Vec::with_capacity(count);

    for _ in 0..count {
        let (type_pos, type_name) = find_next_type(data, cursor, names)?;

        // The property's name FName is exactly 8 bytes after its type FName.
        let name_pos = type_pos + 8;
        if name_pos + 8 > data.len() {
            return None;
        }

        let name_idx = u32::from_le_bytes(data[name_pos..name_pos + 4].try_into().ok()?) as usize;
        let name_num = u32::from_le_bytes(data[name_pos + 4..name_pos + 8].try_into().ok()?);

        let base = names.get(name_idx)?;
        let mut field_name = base.clone();
        if name_num > 0 {
            field_name.push('_');
            field_name.push_str(&(name_num - 1).to_string());
        }

        raw.push(FieldDefinition {
            field_type: type_name,
            field_name,
            array_dim: 1,
            property_flags: 0,
            struct_type: None,
            inner_type: None,
            key_type: None,
            value_type: None,
        });

        cursor = type_pos + MIN_FIELD_SKIP;
    }

    if raw.len() != count {
        return None;
    }

    Some(strip_inner_duplicates(raw))
}

/// Container properties (Map/Array/Set/Enum) emit a follow-on FField for their
/// inner type that re-uses the parent's `field_name`. Drop those duplicates so
/// the caller sees a single entry per declared variable.
fn strip_inner_duplicates(input: Vec<FieldDefinition>) -> Vec<FieldDefinition> {
    let mut out = Vec::with_capacity(input.len());
    let mut sentinel: Option<String> = None;

    for field in input {
        if let Some(skip_name) = &sentinel {
            if &field.field_name == skip_name {
                continue;
            }
            sentinel = None;
        }

        if matches!(
            field.field_type.as_str(),
            "MapProperty" | "ArrayProperty" | "SetProperty" | "EnumProperty"
        ) {
            sentinel = Some(field.field_name.clone());
        }

        out.push(field);
    }
    out
}

/// Linear scan for the next FName whose value resolves to a known FField
/// class. Type FNames never carry a numeric suffix, so `name_num` must be 0.
fn find_next_type(data: &[u8], from: usize, names: &[String]) -> Option<(usize, String)> {
    let scan_end = data.len().saturating_sub(8);
    for pos in from..scan_end {
        let idx = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
        if idx >= names.len() {
            continue;
        }
        let num = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().ok()?);
        if num != 0 {
            continue;
        }
        let candidate = &names[idx];
        if KNOWN_FIELD_TYPES.contains(&candidate.as_str()) {
            return Some((pos, candidate.clone()));
        }
    }
    None
}
