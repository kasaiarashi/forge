//! FField/FProperty definition parser for UE export data.
//!
//! In uncooked (editor) assets, UStruct exports serialize their property
//! definitions via `SerializeProperties()`. This module parses that data
//! to extract variable names, types, and flags — enabling diffs to show
//! "added variable: TestVar (BoolProperty)" instead of "native data changed".

use std::io::{Cursor, Read, Seek, SeekFrom};

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
fn try_parse_fields_at(
    data: &[u8],
    offset: usize,
    count: usize,
    names: &[String],
) -> Option<Vec<FieldDefinition>> {
    let mut cursor = Cursor::new(data);
    cursor.seek(SeekFrom::Start((offset + 4) as u64)).ok()?; // Skip the PropertyCount

    let mut fields = Vec::with_capacity(count);

    for _ in 0..count {
        let field = parse_single_field(&mut cursor, names)?;
        fields.push(field);
    }

    Some(fields)
}

/// Parse a single FField entry from the cursor.
fn parse_single_field(
    cursor: &mut Cursor<&[u8]>,
    names: &[String],
) -> Option<FieldDefinition> {
    // 1. FName PropertyTypeName (the FField class name).
    let field_type = read_fname(cursor, names)?;

    if !KNOWN_FIELD_TYPES.contains(&field_type.as_str()) {
        return None; // Not a valid field type — abort.
    }

    // 2. FField::Serialize():
    //    - FName NamePrivate
    //    - uint32 FlagsPrivate
    //    - bool bHasMetaData (only in uncooked packages)
    //    - TMap<FName, FString> MetaDataMap (if bHasMetaData)
    let field_name = read_fname(cursor, names)?;
    let _flags_private = read_u32(cursor)?;

    // Read and skip metadata (present in uncooked editor assets).
    // Format: u8 bHasMetaData, then if true: TMap<FName, FString>.
    // We try to skip it; if parsing fails, the whole field parse aborts gracefully.
    if let Some(has_metadata) = read_u8(cursor) {
        if has_metadata != 0 {
            let meta_count = read_i32(cursor)?;
            if meta_count < 0 || meta_count > 1000 {
                return None;
            }
            for _ in 0..meta_count {
                let _key = read_fname(cursor, names)?;
                let _value = skip_fstring(cursor)?;
            }
        }
    } else {
        return None;
    }

    // 3. FProperty::Serialize():
    //    - int32 ArrayDim
    //    - int32 ElementSize (deprecated but serialized)
    //    - uint64 PropertyFlags
    //    - int32 DefaultRepIndex (always 0)
    //    - FName RepNotifyFunc
    //    - uint8 BlueprintReplicationCondition
    let array_dim = read_i32(cursor)?;
    let _element_size = read_i32(cursor)?;
    let property_flags = read_u64(cursor)?;
    let _default_rep_index = read_i32(cursor)?;
    let _rep_notify_func = read_fname(cursor, names)?;
    let _bp_replication_condition = read_u8(cursor)?;

    // Sanity checks.
    if array_dim < 0 || array_dim > 1024 {
        return None;
    }

    // 4. Type-specific data.
    let mut struct_type = None;
    let mut inner_type = None;
    let mut key_type = None;
    let mut value_type = None;

    match field_type.as_str() {
        "StructProperty" => {
            // UScriptStruct* Struct — serialized as an object reference.
            struct_type = read_object_reference_name(cursor, names);
        }
        "ObjectProperty" | "ClassProperty" | "SoftObjectProperty" | "SoftClassProperty"
        | "WeakObjectProperty" | "LazyObjectProperty" | "InterfaceProperty" => {
            // UClass* PropertyClass — serialized as an object reference.
            let _class_ref = read_object_reference_name(cursor, names);
        }
        "ArrayProperty" | "SetProperty" | "OptionalProperty" => {
            // FField* Inner — serialized via SerializeSingleField pattern.
            if let Some(inner_field) = parse_single_field(cursor, names) {
                inner_type = Some(inner_field.field_type);
            }
        }
        "MapProperty" => {
            // FField* KeyProp + FField* ValueProp.
            if let Some(key_field) = parse_single_field(cursor, names) {
                key_type = Some(key_field.field_type);
            }
            if let Some(val_field) = parse_single_field(cursor, names) {
                value_type = Some(val_field.field_type);
            }
        }
        "EnumProperty" => {
            // FNumericProperty* UnderlyingProp + UEnum* Enum.
            // UnderlyingProp is a SerializeSingleField (usually ByteProperty or IntProperty).
            let _underlying = parse_single_field(cursor, names);
            // UEnum* — object reference.
            let _enum_ref = read_object_reference_name(cursor, names);
        }
        "DelegateProperty" | "MulticastDelegateProperty"
        | "MulticastInlineDelegateProperty" | "MulticastSparseDelegateProperty" => {
            // UFunction* SignatureFunction — object reference.
            let _func_ref = read_object_reference_name(cursor, names);
        }
        // BoolProperty, numeric properties, string properties — no extra data.
        _ => {}
    }

    Some(FieldDefinition {
        field_type,
        field_name,
        array_dim,
        property_flags,
        struct_type,
        inner_type,
        key_type,
        value_type,
    })
}

/// Read an object reference (serialized as FPackageIndex = i32) and try to resolve its name.
fn read_object_reference_name(cursor: &mut Cursor<&[u8]>, names: &[String]) -> Option<String> {
    let index = read_i32(cursor)?;
    // We can't fully resolve object references without the import/export tables,
    // but we return the index as a string for now.
    if index == 0 {
        Some("None".to_string())
    } else {
        // Just return the index — the caller can resolve it later.
        Some(format!("Ref[{}]", index))
    }
}

// --- Simple binary readers ---

fn read_u8(cursor: &mut Cursor<&[u8]>) -> Option<u8> {
    let mut buf = [0u8; 1];
    cursor.read_exact(&mut buf).ok()?;
    Some(buf[0])
}

fn read_i32(cursor: &mut Cursor<&[u8]>) -> Option<i32> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf).ok()?;
    Some(i32::from_le_bytes(buf))
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> Option<u32> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf).ok()?;
    Some(u32::from_le_bytes(buf))
}

fn read_u64(cursor: &mut Cursor<&[u8]>) -> Option<u64> {
    let mut buf = [0u8; 8];
    cursor.read_exact(&mut buf).ok()?;
    Some(u64::from_le_bytes(buf))
}

/// Skip an FString: read i32 length, then skip that many bytes.
fn skip_fstring(cursor: &mut Cursor<&[u8]>) -> Option<()> {
    let length = read_i32(cursor)?;
    if length == 0 {
        return Some(());
    }
    let byte_count = if length < 0 {
        (-length as usize) * 2 // UTF-16
    } else {
        length as usize // UTF-8/Latin-1
    };
    cursor.seek(SeekFrom::Current(byte_count as i64)).ok()?;
    Some(())
}

/// Read an FName: u32 name table index + u32 number.
fn read_fname(cursor: &mut Cursor<&[u8]>, names: &[String]) -> Option<String> {
    let index = read_u32(cursor)? as usize;
    let number = read_u32(cursor)?;

    if index >= names.len() {
        return None;
    }

    let mut name = names[index].clone();
    if number > 0 {
        name.push_str(&format!("_{}", number - 1));
    }
    Some(name)
}
