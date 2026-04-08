//! FField/FProperty definition parser for UE export data.
//!
//! In uncooked (editor) assets, UStruct exports serialize their property
//! definitions via `SerializeProperties()`. This module parses that data
//! to extract variable names, types, and flags — enabling diffs to show
//! "added variable: TestVar (BoolProperty)" instead of "native data changed".

// No cursor-based reads — we use direct byte slice access for robustness across UE versions.

/// A parsed property definition from a UStruct's field list.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDefinition {
    /// The FField class name (e.g., "BoolProperty", "IntProperty", "StructProperty").
    pub field_type: String,
    /// The property name (e.g., "TestVar", "Health", "MovementSpeed").
    pub field_name: String,
    /// Array dimension (1 for scalar, >1 for fixed-size arrays).
    pub array_dim: i32,
    /// Property flags (CPF_BlueprintVisible, CPF_Edit, etc.).
    pub property_flags: u64,
    /// For StructProperty: the struct type name.
    pub struct_type: Option<String>,
    /// For ArrayProperty/SetProperty: the inner type name.
    pub inner_type: Option<String>,
    /// For MapProperty: the key and value type names.
    pub key_type: Option<String>,
    pub value_type: Option<String>,
}

impl std::fmt::Display for FieldDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let type_detail = if let Some(ref st) = self.struct_type {
            format!("{}<{}>", self.field_type, st)
        } else if let Some(ref it) = self.inner_type {
            format!("{}<{}>", self.field_type, it)
        } else if let (Some(kt), Some(vt)) = (&self.key_type, &self.value_type) {
            format!("{}<{}, {}>", self.field_type, kt, vt)
        } else {
            self.field_type.clone()
        };

        if self.array_dim > 1 {
            write!(f, "{} {}[{}]", type_detail, self.field_name, self.array_dim)
        } else {
            write!(f, "{} {}", type_detail, self.field_name)
        }
    }
}

/// Try to extract property definitions from a UStruct/UClass export's data.
///
/// Scans the export data for the `SerializeProperties()` pattern:
/// `int32 PropertyCount` followed by `PropertyCount` FField entries.
///
/// Returns `None` if the data can't be parsed (not a struct/class, or format unknown).
pub fn parse_field_definitions(
    export_data: &[u8],
    names: &[String],
    class_name: &str,
) -> Option<Vec<FieldDefinition>> {
    // Only attempt for class/struct exports that would have property definitions.
    if !is_class_export(class_name) {
        return None;
    }

    // Strategy: scan for a plausible PropertyCount followed by valid FField entries.
    // The property definitions appear after the UStruct header (SuperStruct, Children)
    // but before script bytecode. We look for a small positive int32 followed by
    // valid FName property type names.
    scan_for_property_definitions(export_data, names)
}

/// Check if this export's class is one that serializes property definitions.
fn is_class_export(class_name: &str) -> bool {
    class_name == "Class"
        || class_name == "ScriptStruct"
        || class_name.ends_with("GeneratedClass")
        || class_name == "BlueprintGeneratedClass"
        || class_name == "WidgetBlueprintGeneratedClass"
        || class_name == "AnimBlueprintGeneratedClass"
}

/// Known FField type names that appear in SerializeProperties().
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

/// Scan export data for the PropertyCount + FField entries pattern.
fn scan_for_property_definitions(data: &[u8], names: &[String]) -> Option<Vec<FieldDefinition>> {
    if data.len() < 8 {
        return None;
    }

    // Search for a valid PropertyCount (small positive number) followed by
    // an FName that matches a known field type. We scan forward through the
    // data looking for this pattern.
    let mut best_result: Option<Vec<FieldDefinition>> = None;
    let mut best_count = 0usize;

    for offset in 0..data.len().saturating_sub(12) {
        let Ok(count_bytes) = data[offset..offset + 4].try_into() else { continue };
        let count = i32::from_le_bytes(count_bytes);

        // PropertyCount should be reasonable (2-500 for Blueprint classes).
        // We require at least 2 to avoid false positives from random i32 values.
        if count < 2 || count > 500 {
            continue;
        }

        // Skip if this count is smaller than what we've already found.
        if (count as usize) <= best_count {
            continue;
        }

        // Check if the next 4 bytes are a valid FName index pointing to a known field type.
        let Ok(idx_bytes) = data[offset + 4..offset + 8].try_into() else { continue };
        let name_idx = u32::from_le_bytes(idx_bytes) as usize;
        if name_idx >= names.len() {
            continue;
        }

        let first_type = &names[name_idx];
        if !KNOWN_FIELD_TYPES.contains(&first_type.as_str()) {
            continue;
        }

        // This looks like a valid PropertyCount + first field type. Try to parse all entries.
        if let Some(fields) = try_parse_fields_at(data, offset, count as usize, names) {
            if fields.len() > best_count {
                best_count = fields.len();
                best_result = Some(fields);
            }
        }
    }

    best_result
}

/// Try to parse `count` FField entries starting from `offset` in the data.
///
/// Uses a scan-based approach: find each FField by looking for the next known
/// property type FName after the current position. This is more robust than
/// trying to parse every byte of FProperty data (which varies by UE version).
fn try_parse_fields_at(
    data: &[u8],
    offset: usize,
    count: usize,
    names: &[String],
) -> Option<Vec<FieldDefinition>> {
    let start = offset + 4; // Skip the PropertyCount
    let mut fields = Vec::with_capacity(count);

    // Find positions of all property type FNames in sequence.
    let mut search_pos = start;

    for _field_idx in 0..count {
        // Find the next known field type FName at or after search_pos.
        let (type_pos, field_type) = find_next_field_type(data, search_pos, names)?;

        // The field name FName immediately follows the type FName (8 bytes after).
        let name_pos = type_pos + 8;
        if name_pos + 8 > data.len() {
            return None;
        }

        let name_idx = u32::from_le_bytes(data[name_pos..name_pos + 4].try_into().ok()?) as usize;
        let name_num = u32::from_le_bytes(data[name_pos + 4..name_pos + 8].try_into().ok()?);

        if name_idx >= names.len() {
            return None;
        }

        let mut field_name = names[name_idx].clone();
        if name_num > 0 {
            field_name.push_str(&format!("_{}", name_num - 1));
        }

        fields.push(FieldDefinition {
            field_type: field_type.clone(),
            field_name,
            array_dim: 1,
            property_flags: 0,
            struct_type: None,
            inner_type: None,
            key_type: None,
            value_type: None,
        });

        // Skip past this field to find the next one.
        // Minimum skip: type FName(8) + name FName(8) = 16 bytes.
        search_pos = name_pos + 8;
    }

    if fields.len() == count {
        Some(fields)
    } else {
        None
    }
}

/// Find the next known property type FName at or after `start` in the data.
fn find_next_field_type(data: &[u8], start: usize, names: &[String]) -> Option<(usize, String)> {
    for pos in start..data.len().saturating_sub(8) {
        let name_idx = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
        if name_idx >= names.len() { continue; }

        let name_num = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().ok()?);
        if name_num != 0 { continue; } // Type names never have a number suffix

        let name = &names[name_idx];
        if KNOWN_FIELD_TYPES.contains(&name.as_str()) {
            return Some((pos, name.clone()));
        }
    }
    None
}
