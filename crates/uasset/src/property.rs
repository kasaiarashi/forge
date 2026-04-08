//! Tagged property parser for UE asset export data.
//!
//! UE serializes UObject properties using a tagged format where each property
//! has an FName key, type name, size, and value. This module parses that stream
//! into a structured representation that can be diffed and displayed.

use std::fmt;
use std::io::{Cursor, Read, Seek, SeekFrom};

/// A parsed property value from the tagged property stream.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    Bool(bool),
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    UInt16(u16),
    UInt32(u32),
    UInt64(u64),
    Float(f32),
    Double(f64),
    Str(String),
    Name(String),
    Text(String),
    Object(String),
    SoftObject { path: String, sub_path: String },
    Enum { enum_type: String, value: String },
    Struct {
        struct_type: String,
        fields: Vec<TaggedProperty>,
    },
    Array {
        inner_type: String,
        elements: Vec<PropertyValue>,
    },
    Map {
        key_type: String,
        value_type: String,
        entries: Vec<(PropertyValue, PropertyValue)>,
    },
    Set {
        inner_type: String,
        elements: Vec<PropertyValue>,
    },
    /// Fallback for unknown or unparseable property types.
    Opaque {
        type_name: String,
        data: Vec<u8>,
    },
}

impl fmt::Display for PropertyValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyValue::Bool(v) => write!(f, "{}", v),
            PropertyValue::Int8(v) => write!(f, "{}", v),
            PropertyValue::Int16(v) => write!(f, "{}", v),
            PropertyValue::Int32(v) => write!(f, "{}", v),
            PropertyValue::Int64(v) => write!(f, "{}", v),
            PropertyValue::UInt16(v) => write!(f, "{}", v),
            PropertyValue::UInt32(v) => write!(f, "{}", v),
            PropertyValue::UInt64(v) => write!(f, "{}", v),
            PropertyValue::Float(v) => write!(f, "{:.4}", v),
            PropertyValue::Double(v) => write!(f, "{:.6}", v),
            PropertyValue::Str(v) => write!(f, "\"{}\"", v),
            PropertyValue::Name(v) => write!(f, "{}", v),
            PropertyValue::Text(v) => write!(f, "\"{}\"", v),
            PropertyValue::Object(v) => write!(f, "{}", v),
            PropertyValue::SoftObject { path, sub_path } => {
                if sub_path.is_empty() {
                    write!(f, "{}", path)
                } else {
                    write!(f, "{}:{}", path, sub_path)
                }
            }
            PropertyValue::Enum { value, .. } => write!(f, "{}", value),
            PropertyValue::Struct { struct_type, fields } => {
                write!(f, "{} {{", struct_type)?;
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, " {}: {}", field.name, field.value)?;
                }
                write!(f, " }}")
            }
            PropertyValue::Array { elements, .. } => {
                write!(f, "[")?;
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if i >= 5 {
                        write!(f, "...+{}", elements.len() - 5)?;
                        break;
                    }
                    write!(f, "{}", elem)?;
                }
                write!(f, "]")
            }
            PropertyValue::Map { entries, .. } => {
                write!(f, "{{")?;
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if i >= 3 {
                        write!(f, "...+{}", entries.len() - 3)?;
                        break;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
            PropertyValue::Set { elements, .. } => {
                write!(f, "{{")?;
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if i >= 5 {
                        write!(f, "...+{}", elements.len() - 5)?;
                        break;
                    }
                    write!(f, "{}", elem)?;
                }
                write!(f, "}}")
            }
            PropertyValue::Opaque { type_name, data } => {
                write!(f, "<{}, {} bytes>", type_name, data.len())
            }
        }
    }
}

/// A single tagged property from the export data stream.
#[derive(Debug, Clone, PartialEq)]
pub struct TaggedProperty {
    pub name: String,
    pub type_name: String,
    pub array_index: u32,
    pub value: PropertyValue,
}

/// All parsed properties from a single export object.
#[derive(Debug, Clone)]
pub struct ExportProperties {
    pub export_name: String,
    pub class_name: String,
    pub properties: Vec<TaggedProperty>,
    /// Hash of trailing bytes after the property list (native serialization data).
    pub trailing_data_size: usize,
}

/// Parse tagged properties from a byte slice (the export data region).
///
/// `names` is the asset's name table for resolving FName indices.
/// Returns the list of parsed properties, or an error if the data is malformed.
pub fn parse_tagged_properties(
    data: &[u8],
    names: &[String],
) -> Result<Vec<TaggedProperty>, PropertyParseError> {
    let mut cursor = Cursor::new(data);
    let mut properties = Vec::new();

    loop {
        // Read FName for property name.
        let name = match read_fname(&mut cursor, names) {
            Ok(n) => n,
            Err(_) => break, // End of data or corrupt — stop gracefully.
        };

        // "None" terminates the property list.
        if name == "None" {
            break;
        }

        // Read FName for type name.
        let type_name = read_fname(&mut cursor, names)?;

        // Read value size (i32) and array index (i32).
        let value_size = read_i32(&mut cursor)?;
        let array_index = read_i32(&mut cursor)? as u32;

        if value_size < 0 {
            return Err(PropertyParseError::InvalidSize(name, value_size));
        }

        // Parse the value based on type.
        let value_start = cursor.position();
        let value = parse_property_value(
            &mut cursor,
            &type_name,
            value_size as usize,
            names,
        );

        // Ensure we consumed exactly the right number of bytes for non-bool types.
        // BoolProperty stores its value in the tag, so value_size is 0 and we read 1 extra byte.
        if type_name != "BoolProperty" {
            let consumed = (cursor.position() - value_start) as usize;
            let expected = value_size as usize;
            if consumed != expected {
                // Seek to the correct position to stay synchronized.
                cursor.seek(SeekFrom::Start(value_start + expected as u64))
                    .map_err(|e| PropertyParseError::Io(e.to_string()))?;
            }
        }

        properties.push(TaggedProperty {
            name,
            type_name,
            array_index,
            value,
        });
    }

    Ok(properties)
}

/// Errors that can occur during property parsing.
#[derive(Debug)]
pub enum PropertyParseError {
    Io(String),
    InvalidNameIndex(u32),
    InvalidSize(String, i32),
    UnexpectedEof,
}

impl fmt::Display for PropertyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyParseError::Io(e) => write!(f, "I/O error: {}", e),
            PropertyParseError::InvalidNameIndex(i) => write!(f, "invalid name index: {}", i),
            PropertyParseError::InvalidSize(name, size) => {
                write!(f, "invalid size {} for property '{}'", size, name)
            }
            PropertyParseError::UnexpectedEof => write!(f, "unexpected end of data"),
        }
    }
}

impl std::error::Error for PropertyParseError {}

// --- Internal helpers ---

fn read_u8(cursor: &mut Cursor<&[u8]>) -> Result<u8, PropertyParseError> {
    let mut buf = [0u8; 1];
    cursor.read_exact(&mut buf).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(buf[0])
}

fn read_i16(cursor: &mut Cursor<&[u8]>) -> Result<i16, PropertyParseError> {
    let mut buf = [0u8; 2];
    cursor.read_exact(&mut buf).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(i16::from_le_bytes(buf))
}

fn read_u16(cursor: &mut Cursor<&[u8]>) -> Result<u16, PropertyParseError> {
    let mut buf = [0u8; 2];
    cursor.read_exact(&mut buf).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(u16::from_le_bytes(buf))
}

fn read_i32(cursor: &mut Cursor<&[u8]>) -> Result<i32, PropertyParseError> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(i32::from_le_bytes(buf))
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> Result<u32, PropertyParseError> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_i64(cursor: &mut Cursor<&[u8]>) -> Result<i64, PropertyParseError> {
    let mut buf = [0u8; 8];
    cursor.read_exact(&mut buf).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(i64::from_le_bytes(buf))
}

fn read_u64(cursor: &mut Cursor<&[u8]>) -> Result<u64, PropertyParseError> {
    let mut buf = [0u8; 8];
    cursor.read_exact(&mut buf).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(u64::from_le_bytes(buf))
}

fn read_f32(cursor: &mut Cursor<&[u8]>) -> Result<f32, PropertyParseError> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(f32::from_le_bytes(buf))
}

fn read_f64(cursor: &mut Cursor<&[u8]>) -> Result<f64, PropertyParseError> {
    let mut buf = [0u8; 8];
    cursor.read_exact(&mut buf).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(f64::from_le_bytes(buf))
}

fn read_bytes(cursor: &mut Cursor<&[u8]>, count: usize) -> Result<Vec<u8>, PropertyParseError> {
    let mut buf = vec![0u8; count];
    cursor.read_exact(&mut buf).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(buf)
}

/// Read an FName: 4-byte name table index + 4-byte number.
fn read_fname(cursor: &mut Cursor<&[u8]>, names: &[String]) -> Result<String, PropertyParseError> {
    let index = read_u32(cursor)? as usize;
    let number = read_u32(cursor)?;

    if index >= names.len() {
        return Err(PropertyParseError::InvalidNameIndex(index as u32));
    }

    let mut name = names[index].clone();
    if number > 0 {
        name.push_str(&format!("_{}", number - 1));
    }
    Ok(name)
}

/// Read a UE serialized FString: i32 length + UTF-8/UTF-16 bytes + null terminator.
fn read_fstring(cursor: &mut Cursor<&[u8]>) -> Result<String, PropertyParseError> {
    let length = read_i32(cursor)?;

    if length == 0 {
        return Ok(String::new());
    }

    if length < 0 {
        // UTF-16 string.
        let char_count = (-length) as usize;
        let byte_count = char_count * 2;
        let bytes = read_bytes(cursor, byte_count)?;
        let u16s: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        // Strip null terminator.
        let end = u16s.iter().position(|&c| c == 0).unwrap_or(u16s.len());
        Ok(String::from_utf16_lossy(&u16s[..end]))
    } else {
        // UTF-8 / Latin-1 string.
        let byte_count = length as usize;
        let bytes = read_bytes(cursor, byte_count)?;
        // Strip null terminator.
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }
}

/// Read 16 bytes as a GUID (just skip for now, used in struct/map type headers).
fn read_guid(cursor: &mut Cursor<&[u8]>) -> Result<[u8; 16], PropertyParseError> {
    let mut buf = [0u8; 16];
    cursor.read_exact(&mut buf).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(buf)
}

/// Parse a single property value given its type name and expected byte size.
fn parse_property_value(
    cursor: &mut Cursor<&[u8]>,
    type_name: &str,
    value_size: usize,
    names: &[String],
) -> PropertyValue {
    let result = parse_property_value_inner(cursor, type_name, value_size, names);
    match result {
        Ok(v) => v,
        Err(_) => {
            // On any parse failure, try to read remaining bytes as opaque.
            let pos = cursor.position() as usize;
            let data_len = cursor.get_ref().len();
            let remaining = if pos < data_len {
                let end = (pos + value_size).min(data_len);
                cursor.get_ref()[pos..end].to_vec()
            } else {
                Vec::new()
            };
            PropertyValue::Opaque {
                type_name: type_name.to_string(),
                data: remaining,
            }
        }
    }
}

fn parse_property_value_inner(
    cursor: &mut Cursor<&[u8]>,
    type_name: &str,
    value_size: usize,
    names: &[String],
) -> Result<PropertyValue, PropertyParseError> {
    match type_name {
        "BoolProperty" => {
            // BoolProperty stores the value as a single byte BEFORE the value_size bytes.
            // value_size is 0 for BoolProperty; the bool is in the tag.
            let val = read_u8(cursor)?;
            Ok(PropertyValue::Bool(val != 0))
        }

        "Int8Property" | "ByteProperty" if value_size == 1 => {
            let val = read_u8(cursor)? as i8;
            Ok(PropertyValue::Int8(val))
        }

        "ByteProperty" => {
            // ByteProperty with size > 1 is an enum stored as FName.
            let enum_type = read_fname(cursor, names)?;
            if value_size == 8 {
                // Just the enum name as an FName (8 bytes).
                Ok(PropertyValue::Enum {
                    enum_type: "ByteProperty".to_string(),
                    value: enum_type,
                })
            } else {
                // Enum with FName type header + value.
                let value = read_fname(cursor, names)?;
                Ok(PropertyValue::Enum {
                    enum_type,
                    value,
                })
            }
        }

        "Int16Property" => {
            let val = read_i16(cursor)?;
            Ok(PropertyValue::Int16(val))
        }

        "IntProperty" => {
            let val = read_i32(cursor)?;
            Ok(PropertyValue::Int32(val))
        }

        "Int64Property" => {
            let val = read_i64(cursor)?;
            Ok(PropertyValue::Int64(val))
        }

        "UInt16Property" => {
            let val = read_u16(cursor)?;
            Ok(PropertyValue::UInt16(val))
        }

        "UInt32Property" => {
            let val = read_u32(cursor)?;
            Ok(PropertyValue::UInt32(val))
        }

        "UInt64Property" => {
            let val = read_u64(cursor)?;
            Ok(PropertyValue::UInt64(val))
        }

        "FloatProperty" => {
            let val = read_f32(cursor)?;
            Ok(PropertyValue::Float(val))
        }

        "DoubleProperty" => {
            let val = read_f64(cursor)?;
            Ok(PropertyValue::Double(val))
        }

        "StrProperty" | "TextProperty" => {
            let val = read_fstring(cursor)?;
            if type_name == "TextProperty" {
                Ok(PropertyValue::Text(val))
            } else {
                Ok(PropertyValue::Str(val))
            }
        }

        "NameProperty" => {
            let val = read_fname(cursor, names)?;
            Ok(PropertyValue::Name(val))
        }

        "ObjectProperty" | "InterfaceProperty" | "LazyObjectProperty" => {
            // Serialized as i32 object reference index.
            let index = read_i32(cursor)?;
            let desc = if index == 0 {
                "None".to_string()
            } else if index > 0 {
                format!("Export[{}]", index - 1)
            } else {
                format!("Import[{}]", -(index + 1))
            };
            Ok(PropertyValue::Object(desc))
        }

        "SoftObjectProperty" => {
            let path = read_fstring(cursor)?;
            let sub_path = read_fstring(cursor)?;
            Ok(PropertyValue::SoftObject { path, sub_path })
        }

        "EnumProperty" => {
            // EnumProperty has an FName type in the tag header (before value bytes).
            let enum_type = read_fname(cursor, names)?;
            // Then a GUID (UE 5+ may have this, but often 0).
            let _guid = read_u8(cursor)?;
            // The value is an FName.
            let value = read_fname(cursor, names)?;
            Ok(PropertyValue::Enum {
                enum_type,
                value,
            })
        }

        "StructProperty" => {
            // StructProperty tag header: FName struct_type + GUID (16 bytes) + HasPropertyGuid (1 byte).
            let struct_type = read_fname(cursor, names)?;
            let _guid = read_guid(cursor)?;
            let _has_prop_guid = read_u8(cursor)?;

            // The struct value is a nested tagged property list for known struct types,
            // or raw bytes for native structs (Vector, Rotator, etc.).
            let fields = parse_struct_value(cursor, &struct_type, value_size, names)?;
            Ok(PropertyValue::Struct {
                struct_type,
                fields,
            })
        }

        "ArrayProperty" => {
            // ArrayProperty tag header: FName inner_type + HasPropertyGuid (1 byte).
            let inner_type = read_fname(cursor, names)?;
            let _has_prop_guid = read_u8(cursor)?;

            // Value: i32 element count, then elements.
            let count = read_i32(cursor)? as usize;
            let mut elements = Vec::with_capacity(count.min(1024));

            for _ in 0..count {
                let elem = parse_array_element(cursor, &inner_type, names)?;
                elements.push(elem);
            }

            Ok(PropertyValue::Array {
                inner_type,
                elements,
            })
        }

        "MapProperty" => {
            // MapProperty tag header: FName key_type + FName value_type + HasPropertyGuid (1 byte).
            let key_type = read_fname(cursor, names)?;
            let value_type = read_fname(cursor, names)?;
            let _has_prop_guid = read_u8(cursor)?;

            // Value: i32 num_keys_to_remove (usually 0), i32 count, then entries.
            let _num_remove = read_i32(cursor)?;
            let count = read_i32(cursor)? as usize;
            let mut entries = Vec::with_capacity(count.min(1024));

            for _ in 0..count {
                let key = parse_array_element(cursor, &key_type, names)?;
                let val = parse_array_element(cursor, &value_type, names)?;
                entries.push((key, val));
            }

            Ok(PropertyValue::Map {
                key_type,
                value_type,
                entries,
            })
        }

        "SetProperty" => {
            // SetProperty tag header: FName inner_type + HasPropertyGuid (1 byte).
            let inner_type = read_fname(cursor, names)?;
            let _has_prop_guid = read_u8(cursor)?;

            let _num_remove = read_i32(cursor)?;
            let count = read_i32(cursor)? as usize;
            let mut elements = Vec::with_capacity(count.min(1024));

            for _ in 0..count {
                let elem = parse_array_element(cursor, &inner_type, names)?;
                elements.push(elem);
            }

            Ok(PropertyValue::Set {
                inner_type,
                elements,
            })
        }

        // Unknown type — store as opaque bytes.
        _ => {
            let data = read_bytes(cursor, value_size)?;
            Ok(PropertyValue::Opaque {
                type_name: type_name.to_string(),
                data,
            })
        }
    }
}

/// Parse a struct value. Well-known native structs (Vector, Rotator, etc.) are
/// parsed specially; generic structs use the tagged property stream recursively.
fn parse_struct_value(
    cursor: &mut Cursor<&[u8]>,
    struct_type: &str,
    value_size: usize,
    names: &[String],
) -> Result<Vec<TaggedProperty>, PropertyParseError> {
    match struct_type {
        "Vector" | "Vector_NetQuantize" | "Vector_NetQuantize100" => {
            // FVector: 3 floats (or 3 doubles for LargeWorldCoordinates in UE5.0+).
            if value_size == 24 {
                // Double precision (UE5 LWC).
                let x = read_f64(cursor)?;
                let y = read_f64(cursor)?;
                let z = read_f64(cursor)?;
                Ok(vec![
                    TaggedProperty { name: "X".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(x) },
                    TaggedProperty { name: "Y".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(y) },
                    TaggedProperty { name: "Z".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(z) },
                ])
            } else {
                let x = read_f32(cursor)?;
                let y = read_f32(cursor)?;
                let z = read_f32(cursor)?;
                Ok(vec![
                    TaggedProperty { name: "X".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(x) },
                    TaggedProperty { name: "Y".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(y) },
                    TaggedProperty { name: "Z".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(z) },
                ])
            }
        }

        "Vector2D" => {
            if value_size == 16 {
                let x = read_f64(cursor)?;
                let y = read_f64(cursor)?;
                Ok(vec![
                    TaggedProperty { name: "X".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(x) },
                    TaggedProperty { name: "Y".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(y) },
                ])
            } else {
                let x = read_f32(cursor)?;
                let y = read_f32(cursor)?;
                Ok(vec![
                    TaggedProperty { name: "X".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(x) },
                    TaggedProperty { name: "Y".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(y) },
                ])
            }
        }

        "Vector4" | "Vector4f" | "Vector4d" => {
            if value_size == 32 {
                let x = read_f64(cursor)?;
                let y = read_f64(cursor)?;
                let z = read_f64(cursor)?;
                let w = read_f64(cursor)?;
                Ok(vec![
                    TaggedProperty { name: "X".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(x) },
                    TaggedProperty { name: "Y".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(y) },
                    TaggedProperty { name: "Z".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(z) },
                    TaggedProperty { name: "W".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(w) },
                ])
            } else {
                let x = read_f32(cursor)?;
                let y = read_f32(cursor)?;
                let z = read_f32(cursor)?;
                let w = read_f32(cursor)?;
                Ok(vec![
                    TaggedProperty { name: "X".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(x) },
                    TaggedProperty { name: "Y".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(y) },
                    TaggedProperty { name: "Z".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(z) },
                    TaggedProperty { name: "W".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(w) },
                ])
            }
        }

        "Rotator" => {
            if value_size == 24 {
                let pitch = read_f64(cursor)?;
                let yaw = read_f64(cursor)?;
                let roll = read_f64(cursor)?;
                Ok(vec![
                    TaggedProperty { name: "Pitch".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(pitch) },
                    TaggedProperty { name: "Yaw".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(yaw) },
                    TaggedProperty { name: "Roll".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(roll) },
                ])
            } else {
                let pitch = read_f32(cursor)?;
                let yaw = read_f32(cursor)?;
                let roll = read_f32(cursor)?;
                Ok(vec![
                    TaggedProperty { name: "Pitch".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(pitch) },
                    TaggedProperty { name: "Yaw".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(yaw) },
                    TaggedProperty { name: "Roll".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(roll) },
                ])
            }
        }

        "Quat" | "Quat4f" | "Quat4d" => {
            if value_size == 32 {
                let x = read_f64(cursor)?;
                let y = read_f64(cursor)?;
                let z = read_f64(cursor)?;
                let w = read_f64(cursor)?;
                Ok(vec![
                    TaggedProperty { name: "X".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(x) },
                    TaggedProperty { name: "Y".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(y) },
                    TaggedProperty { name: "Z".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(z) },
                    TaggedProperty { name: "W".into(), type_name: "DoubleProperty".into(), array_index: 0, value: PropertyValue::Double(w) },
                ])
            } else {
                let x = read_f32(cursor)?;
                let y = read_f32(cursor)?;
                let z = read_f32(cursor)?;
                let w = read_f32(cursor)?;
                Ok(vec![
                    TaggedProperty { name: "X".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(x) },
                    TaggedProperty { name: "Y".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(y) },
                    TaggedProperty { name: "Z".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(z) },
                    TaggedProperty { name: "W".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(w) },
                ])
            }
        }

        "Transform" => {
            // FTransform: Rotation (Quat) + Translation (Vector) + Scale3D (Vector).
            // This is a tagged property stream in most cases.
            parse_tagged_properties_from_cursor(cursor, names, value_size)
        }

        "LinearColor" => {
            let r = read_f32(cursor)?;
            let g = read_f32(cursor)?;
            let b = read_f32(cursor)?;
            let a = read_f32(cursor)?;
            Ok(vec![
                TaggedProperty { name: "R".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(r) },
                TaggedProperty { name: "G".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(g) },
                TaggedProperty { name: "B".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(b) },
                TaggedProperty { name: "A".into(), type_name: "FloatProperty".into(), array_index: 0, value: PropertyValue::Float(a) },
            ])
        }

        "Color" => {
            let b = read_u8(cursor)?;
            let g = read_u8(cursor)?;
            let r = read_u8(cursor)?;
            let a = read_u8(cursor)?;
            Ok(vec![
                TaggedProperty { name: "R".into(), type_name: "ByteProperty".into(), array_index: 0, value: PropertyValue::Int8(r as i8) },
                TaggedProperty { name: "G".into(), type_name: "ByteProperty".into(), array_index: 0, value: PropertyValue::Int8(g as i8) },
                TaggedProperty { name: "B".into(), type_name: "ByteProperty".into(), array_index: 0, value: PropertyValue::Int8(b as i8) },
                TaggedProperty { name: "A".into(), type_name: "ByteProperty".into(), array_index: 0, value: PropertyValue::Int8(a as i8) },
            ])
        }

        "IntPoint" => {
            let x = read_i32(cursor)?;
            let y = read_i32(cursor)?;
            Ok(vec![
                TaggedProperty { name: "X".into(), type_name: "IntProperty".into(), array_index: 0, value: PropertyValue::Int32(x) },
                TaggedProperty { name: "Y".into(), type_name: "IntProperty".into(), array_index: 0, value: PropertyValue::Int32(y) },
            ])
        }

        "Guid" => {
            let data = read_bytes(cursor, 16)?;
            let hex = data.iter().map(|b| format!("{:02x}", b)).collect::<String>();
            Ok(vec![
                TaggedProperty { name: "Value".into(), type_name: "Guid".into(), array_index: 0, value: PropertyValue::Str(hex) },
            ])
        }

        "DateTime" => {
            let ticks = read_i64(cursor)?;
            Ok(vec![
                TaggedProperty { name: "Ticks".into(), type_name: "Int64Property".into(), array_index: 0, value: PropertyValue::Int64(ticks) },
            ])
        }

        "Timespan" => {
            let ticks = read_i64(cursor)?;
            Ok(vec![
                TaggedProperty { name: "Ticks".into(), type_name: "Int64Property".into(), array_index: 0, value: PropertyValue::Int64(ticks) },
            ])
        }

        "SoftObjectPath" | "SoftClassPath" | "StringAssetReference" | "StringClassReference" => {
            let path = read_fstring(cursor)?;
            Ok(vec![
                TaggedProperty { name: "Path".into(), type_name: "StrProperty".into(), array_index: 0, value: PropertyValue::Str(path) },
            ])
        }

        "GameplayTag" => {
            let tag = read_fname(cursor, names)?;
            Ok(vec![
                TaggedProperty { name: "TagName".into(), type_name: "NameProperty".into(), array_index: 0, value: PropertyValue::Name(tag) },
            ])
        }

        "GameplayTagContainer" => {
            let count = read_i32(cursor)? as usize;
            let mut tags = Vec::with_capacity(count.min(256));
            for _ in 0..count {
                let tag = read_fname(cursor, names)?;
                tags.push(PropertyValue::Name(tag));
            }
            Ok(vec![
                TaggedProperty {
                    name: "GameplayTags".into(),
                    type_name: "ArrayProperty".into(),
                    array_index: 0,
                    value: PropertyValue::Array { inner_type: "NameProperty".into(), elements: tags },
                },
            ])
        }

        // Generic struct: try to parse as a tagged property stream.
        _ => {
            parse_tagged_properties_from_cursor(cursor, names, value_size)
        }
    }
}

/// Parse tagged properties from current cursor position, limited by value_size.
fn parse_tagged_properties_from_cursor(
    cursor: &mut Cursor<&[u8]>,
    names: &[String],
    value_size: usize,
) -> Result<Vec<TaggedProperty>, PropertyParseError> {
    let start = cursor.position() as usize;
    let end = start + value_size;
    let data = cursor.get_ref();

    if end > data.len() {
        return Err(PropertyParseError::UnexpectedEof);
    }

    let slice = &data[start..end];
    let result = parse_tagged_properties(slice, names);

    // Advance cursor past the struct data regardless of parse result.
    cursor.seek(SeekFrom::Start(end as u64))
        .map_err(|e| PropertyParseError::Io(e.to_string()))?;

    result
}

/// Parse a single array/map/set element value based on inner type name.
fn parse_array_element(
    cursor: &mut Cursor<&[u8]>,
    inner_type: &str,
    names: &[String],
) -> Result<PropertyValue, PropertyParseError> {
    match inner_type {
        "BoolProperty" => {
            let val = read_u8(cursor)?;
            Ok(PropertyValue::Bool(val != 0))
        }
        "Int8Property" | "ByteProperty" => {
            let val = read_u8(cursor)? as i8;
            Ok(PropertyValue::Int8(val))
        }
        "Int16Property" => {
            let val = read_i16(cursor)?;
            Ok(PropertyValue::Int16(val))
        }
        "IntProperty" => {
            let val = read_i32(cursor)?;
            Ok(PropertyValue::Int32(val))
        }
        "Int64Property" => {
            let val = read_i64(cursor)?;
            Ok(PropertyValue::Int64(val))
        }
        "UInt16Property" => {
            let val = read_u16(cursor)?;
            Ok(PropertyValue::UInt16(val))
        }
        "UInt32Property" => {
            let val = read_u32(cursor)?;
            Ok(PropertyValue::UInt32(val))
        }
        "UInt64Property" => {
            let val = read_u64(cursor)?;
            Ok(PropertyValue::UInt64(val))
        }
        "FloatProperty" => {
            let val = read_f32(cursor)?;
            Ok(PropertyValue::Float(val))
        }
        "DoubleProperty" => {
            let val = read_f64(cursor)?;
            Ok(PropertyValue::Double(val))
        }
        "StrProperty" | "TextProperty" => {
            let val = read_fstring(cursor)?;
            Ok(PropertyValue::Str(val))
        }
        "NameProperty" => {
            let val = read_fname(cursor, names)?;
            Ok(PropertyValue::Name(val))
        }
        "ObjectProperty" | "InterfaceProperty" => {
            let index = read_i32(cursor)?;
            let desc = if index == 0 {
                "None".to_string()
            } else if index > 0 {
                format!("Export[{}]", index - 1)
            } else {
                format!("Import[{}]", -(index + 1))
            };
            Ok(PropertyValue::Object(desc))
        }
        "SoftObjectProperty" => {
            let path = read_fstring(cursor)?;
            let sub_path = read_fstring(cursor)?;
            Ok(PropertyValue::SoftObject { path, sub_path })
        }
        "EnumProperty" => {
            let val = read_fname(cursor, names)?;
            Ok(PropertyValue::Enum {
                enum_type: inner_type.to_string(),
                value: val,
            })
        }
        "StructProperty" => {
            // Array of structs: each element is a tagged property list terminated by "None".
            // But we don't know the size per element, so we parse the property stream.
            let mut props = Vec::new();
            loop {
                let name = read_fname(cursor, names)?;
                if name == "None" {
                    break;
                }
                let type_name = read_fname(cursor, names)?;
                let value_size = read_i32(cursor)? as usize;
                let array_index = read_i32(cursor)? as u32;

                let value_start = cursor.position();
                let value = parse_property_value(cursor, &type_name, value_size, names);

                if type_name != "BoolProperty" {
                    let consumed = (cursor.position() - value_start) as usize;
                    if consumed != value_size {
                        cursor.seek(SeekFrom::Start(value_start + value_size as u64))
                            .map_err(|e| PropertyParseError::Io(e.to_string()))?;
                    }
                }

                props.push(TaggedProperty {
                    name,
                    type_name,
                    array_index,
                    value,
                });
            }
            Ok(PropertyValue::Struct {
                struct_type: "".to_string(),
                fields: props,
            })
        }
        // Unknown inner type — read a single byte and hope for the best.
        _ => {
            Ok(PropertyValue::Opaque {
                type_name: inner_type.to_string(),
                data: Vec::new(),
            })
        }
    }
}

// =============================================================================
// Serialization (inverse of parsing)
// =============================================================================

fn write_u8(buf: &mut Vec<u8>, val: u8) {
    buf.push(val);
}

fn write_i16(buf: &mut Vec<u8>, val: i16) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_u16(buf: &mut Vec<u8>, val: u16) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_i32(buf: &mut Vec<u8>, val: i32) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_u32(buf: &mut Vec<u8>, val: u32) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_i64(buf: &mut Vec<u8>, val: i64) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_u64(buf: &mut Vec<u8>, val: u64) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_f32(buf: &mut Vec<u8>, val: f32) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_f64(buf: &mut Vec<u8>, val: f64) {
    buf.extend_from_slice(&val.to_le_bytes());
}

/// Write an FName: look up `name` in the names table, write (index, number).
/// If `name` has a `_N` suffix where N is a decimal number, the base name is
/// looked up and the number written is N+1. If not found, appends to names.
fn write_fname(buf: &mut Vec<u8>, name: &str, names: &mut Vec<String>) {
    // Try to split off a trailing _N suffix.
    let (base, number) = split_fname_suffix(name);

    let index = match names.iter().position(|n| n == base) {
        Some(i) => i,
        None => {
            let i = names.len();
            names.push(base.to_string());
            i
        }
    };

    write_u32(buf, index as u32);
    write_u32(buf, number);
}

/// Split "Foo_3" into ("Foo", 4) — the number field is suffix+1.
/// If there is no numeric suffix, returns (name, 0).
fn split_fname_suffix(name: &str) -> (&str, u32) {
    if let Some(pos) = name.rfind('_') {
        let suffix = &name[pos + 1..];
        if let Ok(n) = suffix.parse::<u32>() {
            return (&name[..pos], n + 1);
        }
    }
    (name, 0)
}

/// Write a UE serialized FString: i32 length + bytes + null terminator.
/// Empty string writes length 0 with no payload.
fn write_fstring(buf: &mut Vec<u8>, s: &str) {
    if s.is_empty() {
        write_i32(buf, 0);
    } else {
        // Length includes the null terminator.
        let len = s.len() as i32 + 1;
        write_i32(buf, len);
        buf.extend_from_slice(s.as_bytes());
        buf.push(0); // null terminator
    }
}

/// Write 16 bytes of GUID data.
fn write_guid(buf: &mut Vec<u8>, data: &[u8; 16]) {
    buf.extend_from_slice(data);
}

/// Parse an object reference string back to i32 index.
/// "None" -> 0, "Export[N]" -> N+1, "Import[N]" -> -(N+1).
fn parse_object_ref(desc: &str) -> i32 {
    if desc == "None" {
        return 0;
    }
    if let Some(inner) = desc.strip_prefix("Export[").and_then(|s| s.strip_suffix(']')) {
        if let Ok(n) = inner.parse::<i32>() {
            return n + 1;
        }
    }
    if let Some(inner) = desc.strip_prefix("Import[").and_then(|s| s.strip_suffix(']')) {
        if let Ok(n) = inner.parse::<i32>() {
            return -(n + 1);
        }
    }
    0
}

/// Serialize a list of tagged properties into bytes (inverse of `parse_tagged_properties`).
///
/// `names` is the mutable name table — new names encountered will be appended.
/// Returns the serialized byte buffer.
pub fn serialize_tagged_properties(props: &[TaggedProperty], names: &mut Vec<String>) -> Vec<u8> {
    let mut buf = Vec::new();

    for prop in props {
        // Write FName for property name.
        write_fname(&mut buf, &prop.name, names);
        // Write FName for type name.
        write_fname(&mut buf, &prop.type_name, names);

        // For BoolProperty: value_size is 0, and the bool byte goes after array_index.
        if prop.type_name == "BoolProperty" {
            write_i32(&mut buf, 0); // value_size = 0
            write_i32(&mut buf, prop.array_index as i32);
            // Bool byte after array_index.
            let val = match &prop.value {
                PropertyValue::Bool(b) => if *b { 1u8 } else { 0u8 },
                _ => 0u8,
            };
            write_u8(&mut buf, val);
            continue;
        }

        // For other types: write placeholder for value_size, then array_index,
        // then serialize the value, then backpatch the size.
        let size_pos = buf.len();
        write_i32(&mut buf, 0); // placeholder
        write_i32(&mut buf, prop.array_index as i32);

        let value_start = buf.len();
        serialize_property_value(&mut buf, &prop.value, &prop.type_name, names);
        let value_size = (buf.len() - value_start) as i32;

        // Backpatch the size.
        buf[size_pos..size_pos + 4].copy_from_slice(&value_size.to_le_bytes());
    }

    // Write "None" FName terminator.
    write_fname(&mut buf, "None", names);

    buf
}

/// Serialize a single property value based on its type name.
fn serialize_property_value(
    buf: &mut Vec<u8>,
    value: &PropertyValue,
    type_name: &str,
    names: &mut Vec<String>,
) {
    match type_name {
        "BoolProperty" => {
            // Handled in the main loop — should not reach here.
        }

        "Int8Property" => {
            if let PropertyValue::Int8(v) = value {
                write_u8(buf, *v as u8);
            }
        }

        "ByteProperty" => {
            match value {
                PropertyValue::Int8(v) => {
                    // Single byte value (value_size == 1).
                    write_u8(buf, *v as u8);
                }
                PropertyValue::Enum { enum_type, value: val } => {
                    if enum_type == "ByteProperty" {
                        // Just the FName value (size 8).
                        write_fname(buf, val, names);
                    } else {
                        // FName enum_type + FName value.
                        write_fname(buf, enum_type, names);
                        write_fname(buf, val, names);
                    }
                }
                _ => {}
            }
        }

        "Int16Property" => {
            if let PropertyValue::Int16(v) = value {
                write_i16(buf, *v);
            }
        }

        "IntProperty" => {
            if let PropertyValue::Int32(v) = value {
                write_i32(buf, *v);
            }
        }

        "Int64Property" => {
            if let PropertyValue::Int64(v) = value {
                write_i64(buf, *v);
            }
        }

        "UInt16Property" => {
            if let PropertyValue::UInt16(v) = value {
                write_u16(buf, *v);
            }
        }

        "UInt32Property" => {
            if let PropertyValue::UInt32(v) = value {
                write_u32(buf, *v);
            }
        }

        "UInt64Property" => {
            if let PropertyValue::UInt64(v) = value {
                write_u64(buf, *v);
            }
        }

        "FloatProperty" => {
            if let PropertyValue::Float(v) = value {
                write_f32(buf, *v);
            }
        }

        "DoubleProperty" => {
            if let PropertyValue::Double(v) = value {
                write_f64(buf, *v);
            }
        }

        "StrProperty" => {
            if let PropertyValue::Str(v) = value {
                write_fstring(buf, v);
            }
        }

        "TextProperty" => {
            if let PropertyValue::Text(v) = value {
                write_fstring(buf, v);
            }
        }

        "NameProperty" => {
            if let PropertyValue::Name(v) = value {
                write_fname(buf, v, names);
            }
        }

        "ObjectProperty" | "InterfaceProperty" | "LazyObjectProperty" => {
            if let PropertyValue::Object(desc) = value {
                let index = parse_object_ref(desc);
                write_i32(buf, index);
            }
        }

        "SoftObjectProperty" => {
            if let PropertyValue::SoftObject { path, sub_path } = value {
                write_fstring(buf, path);
                write_fstring(buf, sub_path);
            }
        }

        "EnumProperty" => {
            if let PropertyValue::Enum { enum_type, value: val } = value {
                // Tag header: FName enum_type + u8 has_prop_guid.
                write_fname(buf, enum_type, names);
                write_u8(buf, 0);
                // Value: FName.
                write_fname(buf, val, names);
            }
        }

        "StructProperty" => {
            if let PropertyValue::Struct { struct_type, fields } = value {
                // Tag header: FName struct_type + 16-byte zero GUID + u8 has_prop_guid.
                write_fname(buf, struct_type, names);
                write_guid(buf, &[0u8; 16]);
                write_u8(buf, 0);
                // Serialize struct fields.
                serialize_struct_value(buf, struct_type, fields, names);
            }
        }

        "ArrayProperty" => {
            if let PropertyValue::Array { inner_type, elements } = value {
                // Tag header: FName inner_type + u8 has_prop_guid.
                write_fname(buf, inner_type, names);
                write_u8(buf, 0);
                // Value: i32 count + elements.
                write_i32(buf, elements.len() as i32);
                for elem in elements {
                    serialize_array_element(buf, inner_type, elem, names);
                }
            }
        }

        "MapProperty" => {
            if let PropertyValue::Map { key_type, value_type, entries } = value {
                // Tag header: FName key_type + FName value_type + u8 has_prop_guid.
                write_fname(buf, key_type, names);
                write_fname(buf, value_type, names);
                write_u8(buf, 0);
                // Value: i32 num_remove (0) + i32 count + entries.
                write_i32(buf, 0);
                write_i32(buf, entries.len() as i32);
                for (k, v) in entries {
                    serialize_array_element(buf, key_type, k, names);
                    serialize_array_element(buf, value_type, v, names);
                }
            }
        }

        "SetProperty" => {
            if let PropertyValue::Set { inner_type, elements } = value {
                // Tag header: FName inner_type + u8 has_prop_guid.
                write_fname(buf, inner_type, names);
                write_u8(buf, 0);
                // Value: i32 num_remove (0) + i32 count + elements.
                write_i32(buf, 0);
                write_i32(buf, elements.len() as i32);
                for elem in elements {
                    serialize_array_element(buf, inner_type, elem, names);
                }
            }
        }

        // Unknown / opaque — write raw bytes.
        _ => {
            if let PropertyValue::Opaque { data, .. } = value {
                buf.extend_from_slice(data);
            }
        }
    }
}

/// Serialize a struct value, mirroring `parse_struct_value`.
fn serialize_struct_value(
    buf: &mut Vec<u8>,
    struct_type: &str,
    fields: &[TaggedProperty],
    names: &mut Vec<String>,
) {
    match struct_type {
        "Vector" | "Vector_NetQuantize" | "Vector_NetQuantize100" => {
            // 3 components: X, Y, Z — floats or doubles based on field type.
            serialize_vector_components(buf, fields, &["X", "Y", "Z"]);
        }

        "Vector2D" => {
            serialize_vector_components(buf, fields, &["X", "Y"]);
        }

        "Vector4" | "Vector4f" | "Vector4d" => {
            serialize_vector_components(buf, fields, &["X", "Y", "Z", "W"]);
        }

        "Rotator" => {
            serialize_vector_components(buf, fields, &["Pitch", "Yaw", "Roll"]);
        }

        "Quat" | "Quat4f" | "Quat4d" => {
            serialize_vector_components(buf, fields, &["X", "Y", "Z", "W"]);
        }

        "LinearColor" => {
            // Always 4 f32: R, G, B, A.
            for comp_name in &["R", "G", "B", "A"] {
                let val = fields.iter().find(|f| f.name == *comp_name)
                    .map(|f| match &f.value {
                        PropertyValue::Float(v) => *v,
                        _ => 0.0,
                    })
                    .unwrap_or(0.0);
                write_f32(buf, val);
            }
        }

        "Color" => {
            // 4 u8 in B, G, R, A order.
            for comp_name in &["B", "G", "R", "A"] {
                let val = fields.iter().find(|f| f.name == *comp_name)
                    .map(|f| match &f.value {
                        PropertyValue::Int8(v) => *v as u8,
                        _ => 0,
                    })
                    .unwrap_or(0);
                write_u8(buf, val);
            }
        }

        "IntPoint" => {
            for comp_name in &["X", "Y"] {
                let val = fields.iter().find(|f| f.name == *comp_name)
                    .map(|f| match &f.value {
                        PropertyValue::Int32(v) => *v,
                        _ => 0,
                    })
                    .unwrap_or(0);
                write_i32(buf, val);
            }
        }

        "Guid" => {
            // Single "Value" field with hex string -> 16 bytes.
            let hex = fields.iter().find(|f| f.name == "Value")
                .and_then(|f| match &f.value {
                    PropertyValue::Str(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or("");
            let bytes = hex_to_bytes(hex);
            if bytes.len() == 16 {
                buf.extend_from_slice(&bytes);
            } else {
                buf.extend_from_slice(&[0u8; 16]);
            }
        }

        "DateTime" | "Timespan" => {
            let ticks = fields.iter().find(|f| f.name == "Ticks")
                .map(|f| match &f.value {
                    PropertyValue::Int64(v) => *v,
                    _ => 0,
                })
                .unwrap_or(0);
            write_i64(buf, ticks);
        }

        "SoftObjectPath" | "SoftClassPath" | "StringAssetReference" | "StringClassReference" => {
            let path = fields.iter().find(|f| f.name == "Path")
                .and_then(|f| match &f.value {
                    PropertyValue::Str(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or("");
            write_fstring(buf, path);
        }

        "GameplayTag" => {
            let tag = fields.iter().find(|f| f.name == "TagName")
                .and_then(|f| match &f.value {
                    PropertyValue::Name(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or("None");
            write_fname(buf, tag, names);
        }

        "GameplayTagContainer" => {
            let tags_field = fields.iter().find(|f| f.name == "GameplayTags");
            if let Some(TaggedProperty { value: PropertyValue::Array { elements, .. }, .. }) = tags_field {
                write_i32(buf, elements.len() as i32);
                for elem in elements {
                    if let PropertyValue::Name(tag) = elem {
                        write_fname(buf, tag, names);
                    } else {
                        write_fname(buf, "None", names);
                    }
                }
            } else {
                write_i32(buf, 0);
            }
        }

        // Transform and generic structs: recursively serialize as tagged properties.
        _ => {
            let data = serialize_tagged_properties(fields, names);
            buf.extend_from_slice(&data);
        }
    }
}

/// Helper to serialize float/double vector components by field name.
fn serialize_vector_components(buf: &mut Vec<u8>, fields: &[TaggedProperty], comp_names: &[&str]) {
    // Determine if doubles based on first field's type.
    let is_double = fields.first()
        .map(|f| matches!(&f.value, PropertyValue::Double(_)))
        .unwrap_or(false);

    for comp_name in comp_names {
        let field = fields.iter().find(|f| f.name == *comp_name);
        if is_double {
            let val = field.map(|f| match &f.value {
                PropertyValue::Double(v) => *v,
                PropertyValue::Float(v) => *v as f64,
                _ => 0.0,
            }).unwrap_or(0.0);
            write_f64(buf, val);
        } else {
            let val = field.map(|f| match &f.value {
                PropertyValue::Float(v) => *v,
                PropertyValue::Double(v) => *v as f32,
                _ => 0.0,
            }).unwrap_or(0.0);
            write_f32(buf, val);
        }
    }
}

/// Serialize a single array/map/set element value, mirroring `parse_array_element`.
fn serialize_array_element(
    buf: &mut Vec<u8>,
    inner_type: &str,
    value: &PropertyValue,
    names: &mut Vec<String>,
) {
    match inner_type {
        "BoolProperty" => {
            if let PropertyValue::Bool(v) = value {
                write_u8(buf, if *v { 1 } else { 0 });
            }
        }
        "Int8Property" | "ByteProperty" => {
            if let PropertyValue::Int8(v) = value {
                write_u8(buf, *v as u8);
            }
        }
        "Int16Property" => {
            if let PropertyValue::Int16(v) = value {
                write_i16(buf, *v);
            }
        }
        "IntProperty" => {
            if let PropertyValue::Int32(v) = value {
                write_i32(buf, *v);
            }
        }
        "Int64Property" => {
            if let PropertyValue::Int64(v) = value {
                write_i64(buf, *v);
            }
        }
        "UInt16Property" => {
            if let PropertyValue::UInt16(v) = value {
                write_u16(buf, *v);
            }
        }
        "UInt32Property" => {
            if let PropertyValue::UInt32(v) = value {
                write_u32(buf, *v);
            }
        }
        "UInt64Property" => {
            if let PropertyValue::UInt64(v) = value {
                write_u64(buf, *v);
            }
        }
        "FloatProperty" => {
            if let PropertyValue::Float(v) = value {
                write_f32(buf, *v);
            }
        }
        "DoubleProperty" => {
            if let PropertyValue::Double(v) = value {
                write_f64(buf, *v);
            }
        }
        "StrProperty" | "TextProperty" => {
            if let PropertyValue::Str(v) = value {
                write_fstring(buf, v);
            }
        }
        "NameProperty" => {
            if let PropertyValue::Name(v) = value {
                write_fname(buf, v, names);
            }
        }
        "ObjectProperty" | "InterfaceProperty" => {
            if let PropertyValue::Object(desc) = value {
                let index = parse_object_ref(desc);
                write_i32(buf, index);
            }
        }
        "SoftObjectProperty" => {
            if let PropertyValue::SoftObject { path, sub_path } = value {
                write_fstring(buf, path);
                write_fstring(buf, sub_path);
            }
        }
        "EnumProperty" => {
            if let PropertyValue::Enum { value: val, .. } = value {
                write_fname(buf, val, names);
            }
        }
        "StructProperty" => {
            // Array of structs: each element is a tagged property list terminated by "None".
            if let PropertyValue::Struct { fields, .. } = value {
                let data = serialize_tagged_properties(fields, names);
                buf.extend_from_slice(&data);
            }
        }
        _ => {
            if let PropertyValue::Opaque { data, .. } = value {
                buf.extend_from_slice(data);
            }
        }
    }
}

/// Convert a hex string to bytes.
fn hex_to_bytes(hex: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let mut chars = hex.chars();
    while let (Some(a), Some(b)) = (chars.next(), chars.next()) {
        let byte = u8::from_str_radix(&format!("{}{}", a, b), 16).unwrap_or(0);
        bytes.push(byte);
    }
    bytes
}
