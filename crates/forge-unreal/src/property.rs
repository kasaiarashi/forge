//! Tagged property tree parser + serializer for export payloads.
//!
//! UE serializes UObject properties as a stream of `(FName name, FName type,
//! i32 value_size, i32 array_index, value_bytes)` records terminated by an
//! `FName == "None"`. This module decodes that tree into [`TaggedProperty`] and
//! also provides the inverse write path for round-tripping.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{Cursor, Read, Seek, SeekFrom};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// One leaf value in the tagged-property tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    SoftObject {
        path: String,
        sub_path: String,
    },
    Enum {
        enum_type: String,
        value: String,
    },
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
    /// Catch-all for unknown or partially decoded data.
    Opaque {
        type_name: String,
        data: Vec<u8>,
    },
}

impl fmt::Display for PropertyValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use PropertyValue::*;
        match self {
            Bool(v) => write!(f, "{}", v),
            Int8(v) => write!(f, "{}", v),
            Int16(v) => write!(f, "{}", v),
            Int32(v) => write!(f, "{}", v),
            Int64(v) => write!(f, "{}", v),
            UInt16(v) => write!(f, "{}", v),
            UInt32(v) => write!(f, "{}", v),
            UInt64(v) => write!(f, "{}", v),
            Float(v) => write!(f, "{:.4}", v),
            Double(v) => write!(f, "{:.6}", v),
            Str(v) | Text(v) => write!(f, "\"{}\"", v),
            Name(v) | Object(v) => write!(f, "{}", v),
            SoftObject { path, sub_path } if sub_path.is_empty() => write!(f, "{}", path),
            SoftObject { path, sub_path } => write!(f, "{}:{}", path, sub_path),
            Enum { value, .. } => write!(f, "{}", value),
            Struct { struct_type, fields } => {
                write!(f, "{} {{", struct_type)?;
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, " {}: {}", field.name, field.value)?;
                }
                write!(f, " }}")
            }
            Array { elements, .. } | Set { elements, .. } => {
                let opener = if matches!(self, Set { .. }) { '{' } else { '[' };
                let closer = if matches!(self, Set { .. }) { '}' } else { ']' };
                write!(f, "{}", opener)?;
                for (i, e) in elements.iter().enumerate() {
                    if i >= 5 {
                        write!(f, ", ...+{}", elements.len() - 5)?;
                        break;
                    }
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", e)?;
                }
                write!(f, "{}", closer)
            }
            Map { entries, .. } => {
                write!(f, "{{")?;
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i >= 3 {
                        write!(f, ", ...+{}", entries.len() - 3)?;
                        break;
                    }
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
            Opaque { type_name, data } => write!(f, "<{}, {} bytes>", type_name, data.len()),
        }
    }
}

/// One property record from the tagged stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaggedProperty {
    pub name: String,
    pub type_name: String,
    pub array_index: u32,
    pub value: PropertyValue,
}

/// Bundle of properties for a single export.
#[derive(Debug, Clone)]
pub struct ExportProperties {
    pub export_name: String,
    pub class_name: String,
    pub properties: Vec<TaggedProperty>,
    pub trailing_data_size: usize,
}

/// Parse failures surfaced from the tagged-property reader.
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
            PropertyParseError::InvalidSize(name, sz) => {
                write!(f, "invalid size {} for property '{}'", sz, name)
            }
            PropertyParseError::UnexpectedEof => write!(f, "unexpected end of data"),
        }
    }
}

impl std::error::Error for PropertyParseError {}

// ---------------------------------------------------------------------------
// Cursor primitives
// ---------------------------------------------------------------------------

type Cur<'a> = Cursor<&'a [u8]>;

macro_rules! prim_reader {
    ($name:ident, $ty:ty, $size:literal) => {
        fn $name(c: &mut Cur<'_>) -> Result<$ty, PropertyParseError> {
            let mut b = [0u8; $size];
            c.read_exact(&mut b).map_err(|_| PropertyParseError::UnexpectedEof)?;
            Ok(<$ty>::from_le_bytes(b))
        }
    };
}

prim_reader!(rd_i16, i16, 2);
prim_reader!(rd_u16, u16, 2);
prim_reader!(rd_i32, i32, 4);
prim_reader!(rd_u32, u32, 4);
prim_reader!(rd_i64, i64, 8);
prim_reader!(rd_u64, u64, 8);
prim_reader!(rd_f32, f32, 4);
prim_reader!(rd_f64, f64, 8);

fn rd_u8(c: &mut Cur<'_>) -> Result<u8, PropertyParseError> {
    let mut b = [0u8; 1];
    c.read_exact(&mut b).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(b[0])
}

fn rd_bytes(c: &mut Cur<'_>, n: usize) -> Result<Vec<u8>, PropertyParseError> {
    let mut buf = vec![0u8; n];
    c.read_exact(&mut buf).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(buf)
}

fn rd_guid(c: &mut Cur<'_>) -> Result<[u8; 16], PropertyParseError> {
    let mut b = [0u8; 16];
    c.read_exact(&mut b).map_err(|_| PropertyParseError::UnexpectedEof)?;
    Ok(b)
}

/// Read an FName (`u32 index, u32 number`) and resolve it against `names`.
fn rd_fname(c: &mut Cur<'_>, names: &[String]) -> Result<String, PropertyParseError> {
    let index = rd_u32(c)? as usize;
    let number = rd_u32(c)?;
    let base = names
        .get(index)
        .ok_or(PropertyParseError::InvalidNameIndex(index as u32))?;

    if number == 0 {
        Ok(base.clone())
    } else {
        let mut out = String::with_capacity(base.len() + 6);
        out.push_str(base);
        out.push('_');
        out.push_str(&(number - 1).to_string());
        Ok(out)
    }
}

/// Read a UE-serialized FString. Negative length signals UTF-16, positive
/// signals UTF-8/Latin-1; both include the terminating NUL in the count.
fn rd_fstring(c: &mut Cur<'_>) -> Result<String, PropertyParseError> {
    let raw = rd_i32(c)?;
    if raw == 0 {
        return Ok(String::new());
    }

    if raw < 0 {
        let chars = (-raw) as usize;
        let bytes = rd_bytes(c, chars * 2)?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect();
        let cut = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        Ok(String::from_utf16_lossy(&units[..cut]))
    } else {
        let bytes = rd_bytes(c, raw as usize)?;
        let cut = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..cut]).into_owned())
    }
}

// ---------------------------------------------------------------------------
// Top-level parser
// ---------------------------------------------------------------------------

/// Parse the tagged-property stream that lives inside an export's payload.
pub fn parse_tagged_properties(
    data: &[u8],
    names: &[String],
) -> Result<Vec<TaggedProperty>, PropertyParseError> {
    let mut cur = Cursor::new(data);
    let mut out = Vec::new();

    loop {
        // A failure here just terminates parsing — the caller can treat that as
        // a clean end-of-stream when the buffer is exhausted.
        let name = match rd_fname(&mut cur, names) {
            Ok(n) => n,
            Err(_) => break,
        };
        if name == "None" {
            break;
        }

        let type_name = rd_fname(&mut cur, names)?;
        let value_size = rd_i32(&mut cur)?;
        let array_index = rd_i32(&mut cur)? as u32;

        if value_size < 0 {
            return Err(PropertyParseError::InvalidSize(name, value_size));
        }

        let value_start = cur.position();
        let value = parse_value(&mut cur, &type_name, value_size as usize, names);

        // BoolProperty stores its byte in the tag header itself (value_size==0),
        // so don't try to align the cursor for it.
        if type_name != "BoolProperty" {
            let consumed = (cur.position() - value_start) as usize;
            if consumed != value_size as usize {
                cur.seek(SeekFrom::Start(value_start + value_size as u64))
                    .map_err(|e| PropertyParseError::Io(e.to_string()))?;
            }
        }

        out.push(TaggedProperty {
            name,
            type_name,
            array_index,
            value,
        });
    }

    Ok(out)
}

/// Outer wrapper that converts unexpected errors into an `Opaque` placeholder
/// while keeping the cursor in a recoverable position.
fn parse_value(c: &mut Cur<'_>, ty: &str, size: usize, names: &[String]) -> PropertyValue {
    match parse_value_strict(c, ty, size, names) {
        Ok(v) => v,
        Err(_) => {
            let pos = c.position() as usize;
            let buf = c.get_ref();
            let leftover = if pos < buf.len() {
                let end = (pos + size).min(buf.len());
                buf[pos..end].to_vec()
            } else {
                Vec::new()
            };
            PropertyValue::Opaque {
                type_name: ty.to_string(),
                data: leftover,
            }
        }
    }
}

fn parse_value_strict(
    c: &mut Cur<'_>,
    ty: &str,
    size: usize,
    names: &[String],
) -> Result<PropertyValue, PropertyParseError> {
    match ty {
        "BoolProperty" => Ok(PropertyValue::Bool(rd_u8(c)? != 0)),

        "Int8Property" => Ok(PropertyValue::Int8(rd_u8(c)? as i8)),

        "ByteProperty" if size == 1 => Ok(PropertyValue::Int8(rd_u8(c)? as i8)),

        "ByteProperty" => {
            // Byte-as-enum: tag carries the enum type; payload is either nothing
            // (size == 8) or the FName value.
            let enum_type = rd_fname(c, names)?;
            if size == 8 {
                Ok(PropertyValue::Enum {
                    enum_type: "ByteProperty".into(),
                    value: enum_type,
                })
            } else {
                let value = rd_fname(c, names)?;
                Ok(PropertyValue::Enum { enum_type, value })
            }
        }

        "Int16Property" => Ok(PropertyValue::Int16(rd_i16(c)?)),
        "IntProperty" => Ok(PropertyValue::Int32(rd_i32(c)?)),
        "Int64Property" => Ok(PropertyValue::Int64(rd_i64(c)?)),
        "UInt16Property" => Ok(PropertyValue::UInt16(rd_u16(c)?)),
        "UInt32Property" => Ok(PropertyValue::UInt32(rd_u32(c)?)),
        "UInt64Property" => Ok(PropertyValue::UInt64(rd_u64(c)?)),
        "FloatProperty" => Ok(PropertyValue::Float(rd_f32(c)?)),
        "DoubleProperty" => Ok(PropertyValue::Double(rd_f64(c)?)),

        "StrProperty" => Ok(PropertyValue::Str(rd_fstring(c)?)),
        "TextProperty" => Ok(PropertyValue::Text(rd_fstring(c)?)),

        "NameProperty" => Ok(PropertyValue::Name(rd_fname(c, names)?)),

        "ObjectProperty" | "InterfaceProperty" | "LazyObjectProperty" => {
            Ok(PropertyValue::Object(format_object_ref(rd_i32(c)?)))
        }

        "SoftObjectProperty" => {
            let path = rd_fstring(c)?;
            let sub_path = rd_fstring(c)?;
            Ok(PropertyValue::SoftObject { path, sub_path })
        }

        "EnumProperty" => {
            let enum_type = rd_fname(c, names)?;
            let _has_prop_guid = rd_u8(c)?;
            let value = rd_fname(c, names)?;
            Ok(PropertyValue::Enum { enum_type, value })
        }

        "StructProperty" => {
            let struct_type = rd_fname(c, names)?;
            let _guid = rd_guid(c)?;
            let _has_prop_guid = rd_u8(c)?;
            let fields = parse_struct(c, &struct_type, size, names)?;
            Ok(PropertyValue::Struct {
                struct_type,
                fields,
            })
        }

        "ArrayProperty" => {
            let inner_type = rd_fname(c, names)?;
            let _has_prop_guid = rd_u8(c)?;
            let count = rd_i32(c)? as usize;
            let mut elements = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                elements.push(parse_element(c, &inner_type, names)?);
            }
            Ok(PropertyValue::Array {
                inner_type,
                elements,
            })
        }

        "MapProperty" => {
            let key_type = rd_fname(c, names)?;
            let value_type = rd_fname(c, names)?;
            let _has_prop_guid = rd_u8(c)?;
            let _to_remove = rd_i32(c)?;
            let count = rd_i32(c)? as usize;
            let mut entries = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                let k = parse_element(c, &key_type, names)?;
                let v = parse_element(c, &value_type, names)?;
                entries.push((k, v));
            }
            Ok(PropertyValue::Map {
                key_type,
                value_type,
                entries,
            })
        }

        "SetProperty" => {
            let inner_type = rd_fname(c, names)?;
            let _has_prop_guid = rd_u8(c)?;
            let _to_remove = rd_i32(c)?;
            let count = rd_i32(c)? as usize;
            let mut elements = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                elements.push(parse_element(c, &inner_type, names)?);
            }
            Ok(PropertyValue::Set {
                inner_type,
                elements,
            })
        }

        _ => Ok(PropertyValue::Opaque {
            type_name: ty.into(),
            data: rd_bytes(c, size)?,
        }),
    }
}

/// Render an i32 object index back as `"None" | "Export[N]" | "Import[N]"`.
fn format_object_ref(idx: i32) -> String {
    match idx.cmp(&0) {
        std::cmp::Ordering::Equal => "None".to_string(),
        std::cmp::Ordering::Greater => format!("Export[{}]", idx - 1),
        std::cmp::Ordering::Less => format!("Import[{}]", -(idx + 1)),
    }
}

/// Build a tagged-property leaf for a primitive `(name, value)` pair.
fn leaf(name: &str, ty: &str, value: PropertyValue) -> TaggedProperty {
    TaggedProperty {
        name: name.into(),
        type_name: ty.into(),
        array_index: 0,
        value,
    }
}

// ---------------------------------------------------------------------------
// Native struct decoders
// ---------------------------------------------------------------------------

/// Decode well-known native structs into pseudo-tagged properties; everything
/// else falls back to a recursive tagged-property scan over the value range.
fn parse_struct(
    c: &mut Cur<'_>,
    ty: &str,
    size: usize,
    names: &[String],
) -> Result<Vec<TaggedProperty>, PropertyParseError> {
    /// Read N floats or doubles depending on `is_double`, emitting one tagged
    /// property per component name.
    fn read_components(
        c: &mut Cur<'_>,
        is_double: bool,
        comps: &[&str],
    ) -> Result<Vec<TaggedProperty>, PropertyParseError> {
        let mut out = Vec::with_capacity(comps.len());
        for name in comps {
            if is_double {
                out.push(leaf(name, "DoubleProperty", PropertyValue::Double(rd_f64(c)?)));
            } else {
                out.push(leaf(name, "FloatProperty", PropertyValue::Float(rd_f32(c)?)));
            }
        }
        Ok(out)
    }

    match ty {
        "Vector" | "Vector_NetQuantize" | "Vector_NetQuantize100" => {
            read_components(c, size == 24, &["X", "Y", "Z"])
        }
        "Vector2D" => read_components(c, size == 16, &["X", "Y"]),
        "Vector4" | "Vector4f" | "Vector4d" => {
            read_components(c, size == 32, &["X", "Y", "Z", "W"])
        }
        "Rotator" => read_components(c, size == 24, &["Pitch", "Yaw", "Roll"]),
        "Quat" | "Quat4f" | "Quat4d" => read_components(c, size == 32, &["X", "Y", "Z", "W"]),

        "LinearColor" => Ok(vec![
            leaf("R", "FloatProperty", PropertyValue::Float(rd_f32(c)?)),
            leaf("G", "FloatProperty", PropertyValue::Float(rd_f32(c)?)),
            leaf("B", "FloatProperty", PropertyValue::Float(rd_f32(c)?)),
            leaf("A", "FloatProperty", PropertyValue::Float(rd_f32(c)?)),
        ]),

        "Color" => {
            // Wire order is BGRA. Surface the components in RGBA order so
            // consumers don't have to know the swizzle.
            let b = rd_u8(c)?;
            let g = rd_u8(c)?;
            let r = rd_u8(c)?;
            let a = rd_u8(c)?;
            Ok(vec![
                leaf("R", "ByteProperty", PropertyValue::Int8(r as i8)),
                leaf("G", "ByteProperty", PropertyValue::Int8(g as i8)),
                leaf("B", "ByteProperty", PropertyValue::Int8(b as i8)),
                leaf("A", "ByteProperty", PropertyValue::Int8(a as i8)),
            ])
        }

        "IntPoint" => Ok(vec![
            leaf("X", "IntProperty", PropertyValue::Int32(rd_i32(c)?)),
            leaf("Y", "IntProperty", PropertyValue::Int32(rd_i32(c)?)),
        ]),

        "Guid" => {
            let raw = rd_bytes(c, 16)?;
            let mut hex = String::with_capacity(32);
            for byte in &raw {
                hex.push_str(&format!("{:02x}", byte));
            }
            Ok(vec![leaf("Value", "Guid", PropertyValue::Str(hex))])
        }

        "DateTime" | "Timespan" => Ok(vec![leaf(
            "Ticks",
            "Int64Property",
            PropertyValue::Int64(rd_i64(c)?),
        )]),

        "SoftObjectPath" | "SoftClassPath" | "StringAssetReference" | "StringClassReference" => {
            Ok(vec![leaf("Path", "StrProperty", PropertyValue::Str(rd_fstring(c)?))])
        }

        "GameplayTag" => Ok(vec![leaf(
            "TagName",
            "NameProperty",
            PropertyValue::Name(rd_fname(c, names)?),
        )]),

        "GameplayTagContainer" => {
            let n = rd_i32(c)? as usize;
            let mut tags = Vec::with_capacity(n.min(256));
            for _ in 0..n {
                tags.push(PropertyValue::Name(rd_fname(c, names)?));
            }
            Ok(vec![leaf(
                "GameplayTags",
                "ArrayProperty",
                PropertyValue::Array {
                    inner_type: "NameProperty".into(),
                    elements: tags,
                },
            )])
        }

        // Everything else (Transform + arbitrary user structs) is a nested
        // tagged-property block sized by `size`.
        _ => parse_nested_props(c, names, size),
    }
}

/// Carve a `[pos, pos+size)` slice out of the cursor, parse it as a tagged
/// property stream, and unconditionally advance the cursor past the slice.
fn parse_nested_props(
    c: &mut Cur<'_>,
    names: &[String],
    size: usize,
) -> Result<Vec<TaggedProperty>, PropertyParseError> {
    let pos = c.position() as usize;
    let end = pos + size;
    let buf = c.get_ref();

    if end > buf.len() {
        return Err(PropertyParseError::UnexpectedEof);
    }
    let result = parse_tagged_properties(&buf[pos..end], names);
    c.seek(SeekFrom::Start(end as u64))
        .map_err(|e| PropertyParseError::Io(e.to_string()))?;
    result
}

/// Decode one element inside an Array/Map/Set body. Element layouts omit the
/// per-property tag header, so we dispatch on inner type only.
fn parse_element(
    c: &mut Cur<'_>,
    inner_type: &str,
    names: &[String],
) -> Result<PropertyValue, PropertyParseError> {
    match inner_type {
        "BoolProperty" => Ok(PropertyValue::Bool(rd_u8(c)? != 0)),
        "Int8Property" | "ByteProperty" => Ok(PropertyValue::Int8(rd_u8(c)? as i8)),
        "Int16Property" => Ok(PropertyValue::Int16(rd_i16(c)?)),
        "IntProperty" => Ok(PropertyValue::Int32(rd_i32(c)?)),
        "Int64Property" => Ok(PropertyValue::Int64(rd_i64(c)?)),
        "UInt16Property" => Ok(PropertyValue::UInt16(rd_u16(c)?)),
        "UInt32Property" => Ok(PropertyValue::UInt32(rd_u32(c)?)),
        "UInt64Property" => Ok(PropertyValue::UInt64(rd_u64(c)?)),
        "FloatProperty" => Ok(PropertyValue::Float(rd_f32(c)?)),
        "DoubleProperty" => Ok(PropertyValue::Double(rd_f64(c)?)),
        "StrProperty" | "TextProperty" => Ok(PropertyValue::Str(rd_fstring(c)?)),
        "NameProperty" => Ok(PropertyValue::Name(rd_fname(c, names)?)),
        "ObjectProperty" | "InterfaceProperty" => {
            Ok(PropertyValue::Object(format_object_ref(rd_i32(c)?)))
        }
        "SoftObjectProperty" => {
            let path = rd_fstring(c)?;
            let sub_path = rd_fstring(c)?;
            Ok(PropertyValue::SoftObject { path, sub_path })
        }
        "EnumProperty" => Ok(PropertyValue::Enum {
            enum_type: inner_type.into(),
            value: rd_fname(c, names)?,
        }),
        "StructProperty" => {
            // Inside an array we only get a "None"-terminated tagged-property
            // run per element, with no outer size.
            let mut props = Vec::new();
            loop {
                let name = rd_fname(c, names)?;
                if name == "None" {
                    break;
                }
                let type_name = rd_fname(c, names)?;
                let value_size = rd_i32(c)? as usize;
                let array_index = rd_i32(c)? as u32;
                let value_start = c.position();
                let value = parse_value(c, &type_name, value_size, names);
                if type_name != "BoolProperty" {
                    let consumed = (c.position() - value_start) as usize;
                    if consumed != value_size {
                        c.seek(SeekFrom::Start(value_start + value_size as u64))
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
                struct_type: String::new(),
                fields: props,
            })
        }
        _ => Ok(PropertyValue::Opaque {
            type_name: inner_type.into(),
            data: Vec::new(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Serializer
// ---------------------------------------------------------------------------

fn wr_u8(b: &mut Vec<u8>, v: u8) {
    b.push(v);
}
fn wr_i16(b: &mut Vec<u8>, v: i16) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn wr_u16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn wr_i32(b: &mut Vec<u8>, v: i32) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn wr_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn wr_i64(b: &mut Vec<u8>, v: i64) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn wr_u64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn wr_f32(b: &mut Vec<u8>, v: f32) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn wr_f64(b: &mut Vec<u8>, v: f64) {
    b.extend_from_slice(&v.to_le_bytes());
}

/// Inverse of [`rd_fname`]: split off any trailing `_N` suffix, look up (or
/// append) the base in `names`, and emit `(index, number)`.
fn wr_fname(buf: &mut Vec<u8>, full: &str, names: &mut Vec<String>) {
    let (base, number) = split_suffix(full);
    let index = match names.iter().position(|n| n == base) {
        Some(i) => i,
        None => {
            let i = names.len();
            names.push(base.to_string());
            i
        }
    };
    wr_u32(buf, index as u32);
    wr_u32(buf, number);
}

/// `("Foo_3", _)` → `("Foo", 4)`; otherwise `(name, 0)`.
fn split_suffix(name: &str) -> (&str, u32) {
    if let Some(under) = name.rfind('_') {
        if let Ok(parsed) = name[under + 1..].parse::<u32>() {
            return (&name[..under], parsed + 1);
        }
    }
    (name, 0)
}

/// Write an FString (length includes terminator). Always emits Latin-1/UTF-8;
/// non-ASCII payloads are not re-encoded as UTF-16 — that lossiness is
/// inherited from the original implementation.
fn wr_fstring(buf: &mut Vec<u8>, s: &str) {
    if s.is_empty() {
        wr_i32(buf, 0);
        return;
    }
    wr_i32(buf, s.len() as i32 + 1);
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
}

/// Inverse of [`format_object_ref`].
fn parse_object_ref(s: &str) -> i32 {
    if s == "None" {
        return 0;
    }
    if let Some(body) = s.strip_prefix("Export[").and_then(|t| t.strip_suffix(']')) {
        if let Ok(n) = body.parse::<i32>() {
            return n + 1;
        }
    }
    if let Some(body) = s.strip_prefix("Import[").and_then(|t| t.strip_suffix(']')) {
        if let Ok(n) = body.parse::<i32>() {
            return -(n + 1);
        }
    }
    0
}

/// Re-serialize a tagged-property list, growing `names` as new FNames are
/// referenced.
pub fn serialize_tagged_properties(
    props: &[TaggedProperty],
    names: &mut Vec<String>,
) -> Vec<u8> {
    let mut buf = Vec::new();

    for p in props {
        wr_fname(&mut buf, &p.name, names);
        wr_fname(&mut buf, &p.type_name, names);

        if p.type_name == "BoolProperty" {
            // BoolProperty: tag carries the value byte; value_size is zero.
            wr_i32(&mut buf, 0);
            wr_i32(&mut buf, p.array_index as i32);
            let bit = matches!(&p.value, PropertyValue::Bool(true));
            wr_u8(&mut buf, if bit { 1 } else { 0 });
            continue;
        }

        // Reserve a slot for value_size; backpatch once we know the final length.
        let size_slot = buf.len();
        wr_i32(&mut buf, 0);
        wr_i32(&mut buf, p.array_index as i32);

        let body_start = buf.len();
        write_value(&mut buf, &p.value, &p.type_name, names);
        let body_len = (buf.len() - body_start) as i32;
        buf[size_slot..size_slot + 4].copy_from_slice(&body_len.to_le_bytes());
    }

    wr_fname(&mut buf, "None", names);
    buf
}

fn write_value(buf: &mut Vec<u8>, value: &PropertyValue, ty: &str, names: &mut Vec<String>) {
    match ty {
        "BoolProperty" => {} // emitted in the outer loop

        "Int8Property" => {
            if let PropertyValue::Int8(v) = value {
                wr_u8(buf, *v as u8);
            }
        }

        "ByteProperty" => match value {
            PropertyValue::Int8(v) => wr_u8(buf, *v as u8),
            PropertyValue::Enum { enum_type, value: v } => {
                if enum_type == "ByteProperty" {
                    wr_fname(buf, v, names);
                } else {
                    wr_fname(buf, enum_type, names);
                    wr_fname(buf, v, names);
                }
            }
            _ => {}
        },

        "Int16Property" => {
            if let PropertyValue::Int16(v) = value {
                wr_i16(buf, *v);
            }
        }
        "IntProperty" => {
            if let PropertyValue::Int32(v) = value {
                wr_i32(buf, *v);
            }
        }
        "Int64Property" => {
            if let PropertyValue::Int64(v) = value {
                wr_i64(buf, *v);
            }
        }
        "UInt16Property" => {
            if let PropertyValue::UInt16(v) = value {
                wr_u16(buf, *v);
            }
        }
        "UInt32Property" => {
            if let PropertyValue::UInt32(v) = value {
                wr_u32(buf, *v);
            }
        }
        "UInt64Property" => {
            if let PropertyValue::UInt64(v) = value {
                wr_u64(buf, *v);
            }
        }
        "FloatProperty" => {
            if let PropertyValue::Float(v) = value {
                wr_f32(buf, *v);
            }
        }
        "DoubleProperty" => {
            if let PropertyValue::Double(v) = value {
                wr_f64(buf, *v);
            }
        }

        "StrProperty" => {
            if let PropertyValue::Str(s) = value {
                wr_fstring(buf, s);
            }
        }
        "TextProperty" => {
            if let PropertyValue::Text(s) = value {
                wr_fstring(buf, s);
            }
        }
        "NameProperty" => {
            if let PropertyValue::Name(s) = value {
                wr_fname(buf, s, names);
            }
        }

        "ObjectProperty" | "InterfaceProperty" | "LazyObjectProperty" => {
            if let PropertyValue::Object(s) = value {
                wr_i32(buf, parse_object_ref(s));
            }
        }
        "SoftObjectProperty" => {
            if let PropertyValue::SoftObject { path, sub_path } = value {
                wr_fstring(buf, path);
                wr_fstring(buf, sub_path);
            }
        }

        "EnumProperty" => {
            if let PropertyValue::Enum { enum_type, value: v } = value {
                wr_fname(buf, enum_type, names);
                wr_u8(buf, 0);
                wr_fname(buf, v, names);
            }
        }

        "StructProperty" => {
            if let PropertyValue::Struct { struct_type, fields } = value {
                wr_fname(buf, struct_type, names);
                buf.extend_from_slice(&[0u8; 16]);
                wr_u8(buf, 0);
                write_struct(buf, struct_type, fields, names);
            }
        }

        "ArrayProperty" => {
            if let PropertyValue::Array { inner_type, elements } = value {
                wr_fname(buf, inner_type, names);
                wr_u8(buf, 0);
                wr_i32(buf, elements.len() as i32);
                for e in elements {
                    write_element(buf, inner_type, e, names);
                }
            }
        }

        "MapProperty" => {
            if let PropertyValue::Map { key_type, value_type, entries } = value {
                wr_fname(buf, key_type, names);
                wr_fname(buf, value_type, names);
                wr_u8(buf, 0);
                wr_i32(buf, 0); // num_to_remove
                wr_i32(buf, entries.len() as i32);
                for (k, v) in entries {
                    write_element(buf, key_type, k, names);
                    write_element(buf, value_type, v, names);
                }
            }
        }

        "SetProperty" => {
            if let PropertyValue::Set { inner_type, elements } = value {
                wr_fname(buf, inner_type, names);
                wr_u8(buf, 0);
                wr_i32(buf, 0);
                wr_i32(buf, elements.len() as i32);
                for e in elements {
                    write_element(buf, inner_type, e, names);
                }
            }
        }

        _ => {
            if let PropertyValue::Opaque { data, .. } = value {
                buf.extend_from_slice(data);
            }
        }
    }
}

/// Find a struct field by name and project it through `extract`.
fn find_field<'a, T>(
    fields: &'a [TaggedProperty],
    name: &str,
    extract: impl Fn(&'a PropertyValue) -> Option<T>,
) -> Option<T> {
    fields
        .iter()
        .find(|f| f.name == name)
        .and_then(|f| extract(&f.value))
}

fn write_components(
    buf: &mut Vec<u8>,
    fields: &[TaggedProperty],
    comps: &[&str],
) {
    let is_double = matches!(
        fields.first().map(|f| &f.value),
        Some(PropertyValue::Double(_))
    );

    for name in comps {
        if is_double {
            let v = find_field(fields, name, |val| match val {
                PropertyValue::Double(v) => Some(*v),
                PropertyValue::Float(v) => Some(*v as f64),
                _ => None,
            })
            .unwrap_or(0.0);
            wr_f64(buf, v);
        } else {
            let v = find_field(fields, name, |val| match val {
                PropertyValue::Float(v) => Some(*v),
                PropertyValue::Double(v) => Some(*v as f32),
                _ => None,
            })
            .unwrap_or(0.0);
            wr_f32(buf, v);
        }
    }
}

fn write_struct(
    buf: &mut Vec<u8>,
    ty: &str,
    fields: &[TaggedProperty],
    names: &mut Vec<String>,
) {
    match ty {
        "Vector" | "Vector_NetQuantize" | "Vector_NetQuantize100" => {
            write_components(buf, fields, &["X", "Y", "Z"]);
        }
        "Vector2D" => write_components(buf, fields, &["X", "Y"]),
        "Vector4" | "Vector4f" | "Vector4d" => {
            write_components(buf, fields, &["X", "Y", "Z", "W"]);
        }
        "Rotator" => write_components(buf, fields, &["Pitch", "Yaw", "Roll"]),
        "Quat" | "Quat4f" | "Quat4d" => {
            write_components(buf, fields, &["X", "Y", "Z", "W"]);
        }

        "LinearColor" => {
            for n in ["R", "G", "B", "A"] {
                let v = find_field(fields, n, |val| match val {
                    PropertyValue::Float(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(0.0);
                wr_f32(buf, v);
            }
        }

        "Color" => {
            for n in ["B", "G", "R", "A"] {
                let v = find_field(fields, n, |val| match val {
                    PropertyValue::Int8(v) => Some(*v as u8),
                    _ => None,
                })
                .unwrap_or(0);
                wr_u8(buf, v);
            }
        }

        "IntPoint" => {
            for n in ["X", "Y"] {
                let v = find_field(fields, n, |val| match val {
                    PropertyValue::Int32(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(0);
                wr_i32(buf, v);
            }
        }

        "Guid" => {
            let hex = find_field(fields, "Value", |val| match val {
                PropertyValue::Str(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("");
            let bytes = hex_decode(hex);
            if bytes.len() == 16 {
                buf.extend_from_slice(&bytes);
            } else {
                buf.extend_from_slice(&[0u8; 16]);
            }
        }

        "DateTime" | "Timespan" => {
            let ticks = find_field(fields, "Ticks", |val| match val {
                PropertyValue::Int64(v) => Some(*v),
                _ => None,
            })
            .unwrap_or(0);
            wr_i64(buf, ticks);
        }

        "SoftObjectPath" | "SoftClassPath" | "StringAssetReference" | "StringClassReference" => {
            let path = find_field(fields, "Path", |val| match val {
                PropertyValue::Str(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("");
            wr_fstring(buf, path);
        }

        "GameplayTag" => {
            let tag = find_field(fields, "TagName", |val| match val {
                PropertyValue::Name(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("None");
            wr_fname(buf, tag, names);
        }

        "GameplayTagContainer" => match fields.iter().find(|f| f.name == "GameplayTags") {
            Some(TaggedProperty {
                value: PropertyValue::Array { elements, .. },
                ..
            }) => {
                wr_i32(buf, elements.len() as i32);
                for e in elements {
                    if let PropertyValue::Name(t) = e {
                        wr_fname(buf, t, names);
                    } else {
                        wr_fname(buf, "None", names);
                    }
                }
            }
            _ => wr_i32(buf, 0),
        },

        // Transform + arbitrary user structs serialize via the tagged stream.
        _ => {
            let body = serialize_tagged_properties(fields, names);
            buf.extend_from_slice(&body);
        }
    }
}

fn write_element(
    buf: &mut Vec<u8>,
    inner_type: &str,
    value: &PropertyValue,
    names: &mut Vec<String>,
) {
    match inner_type {
        "BoolProperty" => {
            if let PropertyValue::Bool(b) = value {
                wr_u8(buf, if *b { 1 } else { 0 });
            }
        }
        "Int8Property" | "ByteProperty" => {
            if let PropertyValue::Int8(v) = value {
                wr_u8(buf, *v as u8);
            }
        }
        "Int16Property" => {
            if let PropertyValue::Int16(v) = value {
                wr_i16(buf, *v);
            }
        }
        "IntProperty" => {
            if let PropertyValue::Int32(v) = value {
                wr_i32(buf, *v);
            }
        }
        "Int64Property" => {
            if let PropertyValue::Int64(v) = value {
                wr_i64(buf, *v);
            }
        }
        "UInt16Property" => {
            if let PropertyValue::UInt16(v) = value {
                wr_u16(buf, *v);
            }
        }
        "UInt32Property" => {
            if let PropertyValue::UInt32(v) = value {
                wr_u32(buf, *v);
            }
        }
        "UInt64Property" => {
            if let PropertyValue::UInt64(v) = value {
                wr_u64(buf, *v);
            }
        }
        "FloatProperty" => {
            if let PropertyValue::Float(v) = value {
                wr_f32(buf, *v);
            }
        }
        "DoubleProperty" => {
            if let PropertyValue::Double(v) = value {
                wr_f64(buf, *v);
            }
        }
        "StrProperty" | "TextProperty" => {
            if let PropertyValue::Str(s) = value {
                wr_fstring(buf, s);
            }
        }
        "NameProperty" => {
            if let PropertyValue::Name(s) = value {
                wr_fname(buf, s, names);
            }
        }
        "ObjectProperty" | "InterfaceProperty" => {
            if let PropertyValue::Object(s) = value {
                wr_i32(buf, parse_object_ref(s));
            }
        }
        "SoftObjectProperty" => {
            if let PropertyValue::SoftObject { path, sub_path } = value {
                wr_fstring(buf, path);
                wr_fstring(buf, sub_path);
            }
        }
        "EnumProperty" => {
            if let PropertyValue::Enum { value: v, .. } = value {
                wr_fname(buf, v, names);
            }
        }
        "StructProperty" => {
            if let PropertyValue::Struct { fields, .. } = value {
                let body = serialize_tagged_properties(fields, names);
                buf.extend_from_slice(&body);
            }
        }
        _ => {
            if let PropertyValue::Opaque { data, .. } = value {
                buf.extend_from_slice(data);
            }
        }
    }
}

fn hex_decode(hex: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16);
        let lo = (bytes[i + 1] as char).to_digit(16);
        match (hi, lo) {
            (Some(h), Some(l)) => out.push(((h << 4) | l) as u8),
            _ => out.push(0),
        }
        i += 2;
    }
    out
}
