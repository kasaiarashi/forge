# uasset Crate — Rebuild Findings

Comprehensive spec for rebuilding the `uasset` crate from scratch. Parses Unreal Engine `.uasset`/`.umap` package files (UE4.10 → UE5.7+). Header + name table + imports + exports + tagged properties + FField definitions + structured view + round-trip property serialization.

Covers: header + name table + imports + exports + tagged properties + FField definitions + structured view + round-trip property serialization across UE4.10 → UE5.7+.

---

## 1. Crate Manifest

```toml
[package]
name = "uasset"
edition = "2024"
version = "0.6.0"

[dependencies]
binread = "2.1.1"          # little-endian primitive reads (read_le)
bit_field = "0.10.1"       # UCS2->UTF8 bit slicing in parse_string
num-traits = "0.2"         # FromPrimitive for ObjectVersion enums
num-derive = "0.4"
thiserror = "2.0.12"

# commandline-tool feature deps (optional): anyhow, log, simplelog, structopt,
# structopt-flags, tempfile, walkdir, serde, serde_json

[dev-dependencies]
rstest = "0.25.0"
rstest_reuse = "0.7.0"
test_utilities = { path = "test_utilities" }

[features]
commandline-tool = [...]

[[bin]]
name = "uasset"
required-features = ["commandline-tool"]
```

## 2. Module Layout

```
src/
  lib.rs                      # AssetHeader + top-level types + header parse
  archive.rs                  # Archive<R>: file-version state + magic/legacy parse
  enums.rs                    # ObjectVersion, ObjectVersionUE5, PackageFlags, ObjectFlags
  error.rs                    # Error enum (thiserror), Result alias, InvalidNameIndexError
  serialization.rs            # Parseable/Skippable/Deferrable/StreamInfo traits
  serialization/
    implementations.rs        # UnrealString, UnrealArray<T>, UnrealGuid, UnrealEngineVersion,
                              # UnrealObjectExport, UnrealClassImport, UnrealThumbnailInfo, etc.
  property.rs                 # TaggedProperty + PropertyValue + parse/serialize
  ffield.rs                   # FieldDefinition scan/parse (variable names/types)
  structured.rs               # StructuredAsset: combine header+properties+fields
  main.rs                     # CLI (commandline-tool feature)
```

## 3. Core Types (lib.rs)

### `NameReference { index: u32, number: Option<NonZeroU32> }`
Index into `AssetHeader.names`. `number` is 1-based suffix; `Some(1)` means `_0`. `resolve_name` appends `_{number-1}` when set.

### `ObjectReference` (derives from `i32`)
- `0` → `None`
- `>0` → `Export { export_index: (n-1) as usize }`
- `<0` → `Import { import_index: -(n+1) as usize }`

### `ObjectExport` (C++ `FObjectExport`)
Fields: `outer_index, object_name: NameReference, class_index, super_index, template_index, object_flags: u32, serial_size: i64, serial_offset: i64, script_serialization_start_offset: i64, script_serialization_end_offset: i64, forced_export, not_for_client, not_for_server, not_always_loaded_for_editor_game, is_asset, is_inherited_instance, generate_public_hash, package_flags: u32, first_export_dependency, serialization_before_serialization_dependencies, create_before_serialization_dependencies, serialization_before_create_dependencies, create_before_create_dependencies: i32`

Accessors `outer()/class()/superclass()/template()` → `ObjectReference`.

### `ObjectImport` (C++ `FObjectImport`)
`outer_index: i32, object_name, class_package, class_name: NameReference, package_name: Option<NameReference>, import_optional: bool`.

### `ThumbnailInfo`
`object_class_name: String, object_path_without_package_name: String, file_offset: i32`.

### `AssetHeader<R>`
Holds `archive: Archive<R>` plus every field from `FPackageFileSummary` (total_header_size, package_name, package_flags, names, imports/exports, assorted offsets/counts, engine versions, compression_flags, package_source, additional_packages_to_cook, texture_allocations, asset_registry_data_offset, bulk_data_start_offset, world_tile_info_data_offset, chunk_ids, preload_dependency_*, names_referenced_from_export_data_count, payload_toc_offset, data_resource_offset).

### `ImportIterator`
Iterates imports whose `class_name` matches the "Package" name and whose `object_name` is not `/Script/CoreUObject`. Yields dependency package name strings.

## 4. Archive (archive.rs)

Magic: `0x9E2A83C1` (little-endian u32 at offset 0). Any other value → `Error::InvalidFile`.

Constructor `Archive::new(reader)`:
1. Read magic.
2. Read `legacy_version: i32`. Accept range `[-9, -5]`, else `UnsupportedVersion`.
3. Read `_legacy_ue3_version: i32` (discarded).
4. Read `file_version: i32` (UE4 obj version).
5. If `legacy_version <= -8`: read `file_version_ue5: i32`, else 0.
6. Read `file_licensee_version: i32`.
7. If all three version numbers are 0 → `UnversionedAsset`.
8. Parse `file_version` via `ObjectVersion::from_i32` (`UnsupportedUE4Version` on fail).
9. If `file_version_ue5 != 0`: parse `ObjectVersionUE5::from_i32`, but on unknown newer value fall back to highest known `PACKAGE_SAVED_HASH` (forward-compat — UE versions only add fields).

State:
```rust
pub struct Archive<R> {
    pub reader: R,
    pub file_version: ObjectVersion,
    pub file_version_ue5: Option<ObjectVersionUE5>,
    pub file_licensee_version: i32,
    pub legacy_version: i32,
    pub with_editoronly_data: bool,      // set later from package_flags
}
```

Implements `Read + Seek` by delegation. Implements:
- `SerializedObjectVersion<ObjectVersion>`: `self.file_version >= version`
- `SerializedObjectVersion<ObjectVersionUE5>`: `self.file_version_ue5.is_some_and(|v| v >= version)`
- `SerializedFlags`: returns `with_editoronly_data`

`custom_version_serialization_format()`: `legacy_version < -5` → `Optimized`, else `Guids`.

## 5. Errors (error.rs)

```rust
#[derive(Error, Debug)]
pub enum Error {
    InvalidFile,                              // bad magic
    UnsupportedVersion(i32),                  // legacy_version out of range
    UnsupportedUE4Version(i32),
    UnsupportedUE5Version(i32),
    UnversionedAsset,
    ParseFailure(binread::Error),
    Io(std::io::Error),
    InvalidString(std::string::FromUtf8Error),
}

pub struct InvalidNameIndexError(pub u32);
```
`From` impls for `binread::Error` and `io::Error`.

## 6. Version Enums (enums.rs)

### `ObjectVersion` — integer values 214..=522
C++ `EUnrealEngineObjectUE4Version`. Derive `FromPrimitive, PartialOrd, PartialEq, Clone, Copy, Debug`. `#[repr]` is implicit (discriminants given). Full list present in source — preserve exactly for `from_i32` lookups. Key variants referenced in parser:
- `VER_UE4_WORLD_LEVEL_INFO = 224`
- `VER_UE4_ADDED_CHUNKID_TO_ASSETDATA_AND_UPACKAGE = 278`
- `VER_UE4_CHANGED_CHUNKID_TO_BE_AN_ARRAY_OF_CHUNKIDS = 326`
- `VER_UE4_ENGINE_VERSION_OBJECT = 336`
- `VER_UE4_LOAD_FOR_EDITOR_GAME = 365`
- `VER_UE4_ADD_STRING_ASSET_REFERENCES_MAP = 384`
- `VER_UE4_PACKAGE_SUMMARY_HAS_COMPATIBLE_ENGINE_VERSION = 444`
- `VER_UE4_SERIALIZE_TEXT_IN_PACKAGES = 459`
- `VER_UE4_COOKED_ASSETS_IN_EDITOR_SUPPORT = 485`
- `VER_UE4_NAME_HASHES_SERIALIZED = 504`
- `VER_UE4_PRELOAD_DEPENDENCIES_IN_COOKED_EXPORTS = 507`
- `VER_UE4_TemplateIndex_IN_COOKED_EXPORTS = 508`
- `VER_UE4_ADDED_SEARCHABLE_NAMES = 510`
- `VER_UE4_64BIT_EXPORTMAP_SERIALSIZES = 511`
- `VER_UE4_ADDED_PACKAGE_SUMMARY_LOCALIZATION_ID = 516`
- `VER_UE4_ADDED_PACKAGE_OWNER = 518`
- `VER_UE4_NON_OUTER_PACKAGE_IMPORT = 520`
- `VER_UE4_CORRECT_LICENSEE_FLAG = 522` (highest)

### `ObjectVersionUE5` — 1000..=1018
```
INITIAL_VERSION = 1000,
NAMES_REFERENCED_FROM_EXPORT_DATA = 1001,
PAYLOAD_TOC = 1002,
OPTIONAL_RESOURCES = 1003,
LARGE_WORLD_COORDINATES = 1004,
REMOVE_OBJECT_EXPORT_PACKAGE_GUID = 1005,
TRACK_OBJECT_EXPORT_IS_INHERITED = 1006,
FSOFTOBJECTPATH_REMOVE_ASSET_PATH_FNAMES = 1007,
ADD_SOFTOBJECTPATH_LIST = 1008,
DATA_RESOURCES = 1009,
SCRIPT_SERIALIZATION_OFFSET = 1010,
PROPERTY_TAG_EXTENSION_AND_OVERRIDABLE_SERIALIZATION = 1011,
PROPERTY_TAG_COMPLETE_TYPE_NAME = 1012,
ASSETREGISTRY_PACKAGEBUILDDEPENDENCIES = 1013,
METADATA_SERIALIZATION_OFFSET = 1014,
VERSE_CELLS = 1015,
PACKAGE_SAVED_HASH = 1016,
// 1017 unnamed
IMPORT_TYPE_HIERARCHIES = 1018,
```
Use `PartialOrd` for `>=` comparisons.

### `PackageFlags: u32` — full bitflag list. Important: `FilterEditorOnly = 0x80000000`, `UnversionedProperties = 0x00002000`.

### `ObjectFlags: u32` — listed but currently unused except exported for downstream.

## 7. Serialization Traits (serialization.rs)

Generic framework for "parse/skip something at a known offset with a known count". Four traits:

```rust
pub trait ReadInfo: Sized {
    fn get_count(&self) -> u64;
    fn from_current_position<R: Seek + Read + BinReaderExt>(r: &mut R) -> Result<Self>;
}

pub trait StreamInfo: Sized {
    type ReadInfoType: ReadInfo;
    fn get_offset(&self) -> u64;
    fn from_current_position<R: Seek + Read + BinReaderExt>(r: &mut R) -> Result<Self>;
    fn from_indirect_reference<R: Read + BinReaderExt>(r: &mut R) -> Result<Self>;
    fn to_read_info(&self) -> Self::ReadInfoType;
}

pub trait Deferrable { type StreamInfoType: StreamInfo; }

pub trait Parseable: Deferrable {
    type ParsedType: Sized;
    fn parse_with_info_seekless<R>(r: &mut R, ri: &<Self::StreamInfoType as StreamInfo>::ReadInfoType) -> Result<Self::ParsedType>
        where R: Seek + Read + SerializedObjectVersion<ObjectVersion> + SerializedObjectVersion<ObjectVersionUE5> + SerializedFlags;
    // Default methods: parse_with_info, parse_inline, parse_indirect.
}

pub trait Skippable: Deferrable {
    fn seek_past_with_info<R: Seek+Read>(r: &mut R, info: &Self::StreamInfoType) -> Result<()>;
    fn seek_past<R: Seek+Read>(r: &mut R) -> Result<()>;
}
```

Concrete stream-infos:
- `SingleItemStreamInfo { offset: u64 }` with `SingleItemReadInfo {}` (count 1).
- `ArrayStreamInfo { offset: u64, count: u64 }` with `ArrayReadInfo { count: u64 }`.

`from_current_position` for array: read `i32` count at current pos, then record `reader.stream_position()` as offset.
`from_indirect_reference` for array: read `i32 count, i32 offset` (offset is absolute file offset).
`from_indirect_reference` for single-item: read `i32 offset`.

### Blanket impls (implementations.rs top)
- `T: BinRead` → `Deferrable<StreamInfoType=SingleItemStreamInfo>`, `Parseable` (reads `le`), `Skippable` (seeks `offset + size_of::<T>()`).

### Core helpers (implementations.rs)

`skip_string`: read `i32 length`. Negative → UCS-2 (2 bytes/char, length = `-length`). Seek past `length * char_width` bytes.

`parse_string`: same header; for UCS-2 manually convert u16 → UTF-8 bytes via `bit_field::BitField` (3 branches for codepoints <0x80, <0x800, BMP rest). Strip trailing null. For ASCII/UTF-8 read N-1 bytes then skip null.

### `UnrealString`
Parses to `String` via `parse_string`. Skip delegates to `skip_string`.

### `UnrealNameEntryWithHash`
Parse `parse_string` then read `u32` hash (discarded). Skip: `skip_string` + 4 bytes (two u16 hashes).

### `UnrealArray<ElementType>` (PhantomData wrapper)
`StreamInfoType = ArrayStreamInfo`. Parse: `Vec<ElementType::ParsedType>` by looping count times, each element gets its own `ReadInfoType::from_current_position`. Skip: loop, build element stream info, call `ElementType::seek_past_with_info`.

### `UnrealArrayIterator<'a, ElementType, R>`
Lazy iterator over an array in-place. Seeks to `stream_info.offset` on construction.

### Fixed-size skippable structs
- `UnrealGuid`: 16 bytes.
- `UnrealCustomVersion` (optimized): 20 bytes.
- `UnrealGuidCustomVersion` (legacy): 20 bytes + `UnrealString` (friendly name).
- `UnrealGenerationInfo`: 8 bytes.
- `UnrealCompressedChunk`: 16 bytes.

### `UnrealEngineVersion`
```rust
pub struct UnrealEngineVersion {
    pub major: u16, pub minor: u16, pub patch: u16,
    pub changelist: u32, pub is_licensee_version: bool,
    pub branch_name: String,
}
```
`LICENSEE_BIT_MASK = 0x8000_0000`, `CHANGELIST_MASK = 0x7fff_ffff`.
Parse: u16 major, u16 minor, u16 patch, u32 changelist, FString branch_name. Store `changelist & CHANGELIST_MASK`, `is_licensee_version = (changelist & LICENSEE_BIT_MASK) != 0`.
`from_changelist(cl)`: major=4, rest zero, interpret licensee bit.

### `UnrealNameReference`
Parse: read `u32 index`, `u32 raw_number`, store `number = NonZeroU32::new(raw_number)`.

### `UnrealObjectExport` — parse ObjectExport with version gating (see §9).
### `UnrealClassImport` — parse ObjectImport with version gating.
### `UnrealThumbnailInfo` — FString, FString, i32 offset.

## 8. Header Parse Procedure (AssetHeader::new)

Absolutely follow this order — any deviation corrupts subsequent offsets.

1. `Archive::new(reader)` (magic/legacy/versions).

2. If UE5 version `>= PACKAGE_SAVED_HASH` (1016): this is the "new format".
   - Read+discard 20-byte `FIoHash SavedHash`.
   - Read `total_header_size: i32` **here** (moved earlier in new format).

3. Parse+skip `CustomVersionContainer` array:
   - Build `ArrayStreamInfo::from_current_position`.
   - If `custom_version_serialization_format() == Guids`: skip `UnrealArray<UnrealGuidCustomVersion>`.
   - Else: skip `UnrealArray<UnrealCustomVersion>`.

4. If NOT new format: read `total_header_size: i32` now.

5. Read `package_name: UnrealString` (inline FString).

6. Read `package_flags: u32`. Derive `has_editor_only_data = (flags & FilterEditorOnly) == 0`. Store on archive as `with_editoronly_data`.

7. Names table (indirect array):
   - If `file_version >= VER_UE4_NAME_HASHES_SERIALIZED` (504): `UnrealArray::<UnrealNameEntryWithHash>::parse_indirect`.
   - Else: `UnrealArray::<UnrealString>::parse_indirect`.

8. If UE5 `>= ADD_SOFTOBJECTPATH_LIST` (1008): read `(count: i32, offset: i32)` soft object paths; else `(0, 0)`.

9. If `>= VER_UE4_ADDED_PACKAGE_SUMMARY_LOCALIZATION_ID` (516) AND editor-only: read FString `localization_id`.

10. If `>= VER_UE4_SERIALIZE_TEXT_IN_PACKAGES` (459): read `(count, offset: i32)`; else `(0, 0)`.

11. Exports: `UnrealArray::<UnrealObjectExport>::parse_indirect`.
12. Imports: `UnrealArray::<UnrealClassImport>::parse_indirect`.

13. If UE5 `>= VERSE_CELLS` (1015): read 4 x i32 (cell export count/offset, cell import count/offset) — discard.
14. If UE5 `>= METADATA_SERIALIZATION_OFFSET` (1014): read i32 metadata offset — discard.

15. Read `depends_offset: i32`.

16. If `>= VER_UE4_ADD_STRING_ASSET_REFERENCES_MAP` (384): read `(count, offset)`; else `(0, 0)`.

17. If `>= VER_UE4_ADDED_SEARCHABLE_NAMES` (510): read `searchable_names_offset: Some(i32)`; else `None`.

18. Read `thumbnail_table_offset: i32`.

19. If UE5 `>= IMPORT_TYPE_HIERARCHIES` (1018): read 2 x i32 (count, offset) — discard.

20. GUID block:
    - If NOT new format (pre-PACKAGE_SAVED_HASH): skip 16-byte GUID.
    - If `>= VER_UE4_ADDED_PACKAGE_OWNER` (518) AND editor-only: skip 16-byte persistent GUID.
      If `< VER_UE4_NON_OUTER_PACKAGE_IMPORT` (520): also skip 16-byte owner persistent GUID.

21. Generations: read `i32 num_generations`, build `ArrayStreamInfo{offset=pos, count}`, skip `UnrealArray<UnrealGenerationInfo>`.

22. Engine version:
    - If `>= VER_UE4_ENGINE_VERSION_OBJECT` (336): parse `UnrealEngineVersion` inline.
    - Else: read `u32 engine_changelist`, construct via `UnrealEngineVersion::from_changelist` (hardcoded major=4).

23. Compatible engine version:
    - If `>= VER_UE4_PACKAGE_SUMMARY_HAS_COMPATIBLE_ENGINE_VERSION` (444): parse inline.
    - Else: clone `engine_version`.

24. Read `compression_flags: u32`.

25. Compressed chunks: read `i32 num`, build ArrayStreamInfo, skip `UnrealArray<UnrealCompressedChunk>`.

26. Read `package_source: u32`.

27. `additional_packages_to_cook`: parse inline `UnrealArray<UnrealString>`.

28. If `legacy_version > -7`: read `i32 texture_allocations: Some`, else `None`.

29. Read `asset_registry_data_offset: i32`.

30. Read `bulk_data_start_offset: i64`.

31. If `>= VER_UE4_WORLD_LEVEL_INFO` (224): read i32 offset; keep `Some` if >0.

32. Chunk IDs:
    - `has_chunkid = file_version >= VER_UE4_ADDED_CHUNKID_TO_ASSETDATA_AND_UPACKAGE` (278).
    - `has_array = has_chunkid && file_version >= VER_UE4_CHANGED_CHUNKID_TO_BE_AN_ARRAY_OF_CHUNKIDS` (326).
    - If array: parse inline `UnrealArray<i32>`.
    - Elif scalar: read i32, include only if >= 0.
    - Else: empty vec.

33. If `>= VER_UE4_PRELOAD_DEPENDENCIES_IN_COOKED_EXPORTS` (507): read `(preload_dependency_count: i32, preload_dependency_offset: i32)`; else `(-1, 0)`.

34. If UE5 `>= NAMES_REFERENCED_FROM_EXPORT_DATA` (1001): read i32; else `names.len() as i32`.

35. If UE5 `>= PAYLOAD_TOC` (1002): read i64; else `-1`.

36. If UE5 `>= DATA_RESOURCES` (1009): read i32, `Some` if >0; else `None`.

Construct `AssetHeader {...}`.

### `find_name(&str)` — case-insensitive linear search returning `NameReference { index, number: None }`. TODO: handle `_N` suffix stripping.

### `resolve_name(&NameReference)` — look up index; append `_{number-1}` if `number.is_some()`. Returns `Cow<str>` or `InvalidNameIndexError`.

### `package_import_iter()` — ImportIterator filtering imports where `class_name` resolves to "Package" AND `object_name` != "/Script/CoreUObject".

### `thumbnail_iter()` — seeks to `thumbnail_table_offset`, builds `ArrayStreamInfo::from_current_position`, returns `UnrealArrayIterator<UnrealThumbnailInfo, R>`.

## 9. ObjectExport Parse (version gating)

Field order and gates (after `parse_with_info_seekless`):

```
i32 class_index
i32 super_index
if >= VER_UE4_TemplateIndex_IN_COOKED_EXPORTS (508): i32 template_index else 0
i32 outer_index
UnrealNameReference object_name
u32 object_flags

if >= VER_UE4_64BIT_EXPORTMAP_SERIALSIZES (511): i64 serial_size, i64 serial_offset
else: i32→i64 serial_size, i32→i64 serial_offset

u32 forced_export (bool)
u32 not_for_client (bool)
u32 not_for_server (bool)

if NOT >= UE5 REMOVE_OBJECT_EXPORT_PACKAGE_GUID (1005): skip 16-byte GUID
if UE5 >= TRACK_OBJECT_EXPORT_IS_INHERITED (1006): u32 is_inherited_instance (bool) else false

u32 package_flags

if >= VER_UE4_LOAD_FOR_EDITOR_GAME (365): u32 not_always_loaded_for_editor_game (bool) else true
if >= VER_UE4_COOKED_ASSETS_IN_EDITOR_SUPPORT (485): u32 is_asset (bool) else false
if UE5 >= OPTIONAL_RESOURCES (1003): u32 generate_public_hash (bool) else false

if >= VER_UE4_PRELOAD_DEPENDENCIES_IN_COOKED_EXPORTS (507):
    5 x i32: first_export_dependency, serialization_before_serialization,
             create_before_serialization, serialization_before_create,
             create_before_create
else all -1

if UE5 >= SCRIPT_SERIALIZATION_OFFSET (1010):
    i64 script_serialization_start_offset, i64 script_serialization_end_offset
else 0, 0
```

## 10. ObjectImport Parse

```
UnrealNameReference class_package
UnrealNameReference class_name
i32 outer_index
UnrealNameReference object_name
if >= VER_UE4_NON_OUTER_PACKAGE_IMPORT (520) AND editor-only: UnrealNameReference package_name (Some) else None
if UE5 >= OPTIONAL_RESOURCES (1003): u32 import_optional (bool) else false
```

## 11. Tagged Property Parser (property.rs)

### `TaggedProperty { name: String, type_name: String, array_index: u32, value: PropertyValue }`

### `PropertyValue` enum (variants):
`Bool(bool), Int8, Int16, Int32, Int64, UInt16, UInt32, UInt64, Float(f32), Double(f64), Str(String), Name(String), Text(String), Object(String), SoftObject{path, sub_path}, Enum{enum_type, value}, Struct{struct_type, fields: Vec<TaggedProperty>}, Array{inner_type, elements}, Map{key_type, value_type, entries: Vec<(V,V)>}, Set{inner_type, elements}, Opaque{type_name, data: Vec<u8>}`.
`Display` impl truncates arrays at 5, maps at 3.

### FName wire format (inside property data)
`u32 index + u32 number`. Resolve: `names[index]` + (if number>0) `_{number-1}`.

### FString wire format (property.rs `read_fstring`)
`i32 length`. 0 → empty. `<0` → UCS-2: `-length` chars, read `2 * |length|` bytes as u16 LE, lossily UTF-16 decode, strip trailing null. `>0` → UTF-8/Latin-1: read `length` bytes, strip trailing null. **Null terminator is included in length.**

### `parse_tagged_properties(data: &[u8], names: &[String]) -> Result<Vec<TaggedProperty>, PropertyParseError>`
Loop:
1. Read FName `name`. If "None" → break. On read error also break (graceful).
2. Read FName `type_name`.
3. Read `i32 value_size`, `i32 array_index`.
4. `value_size < 0` → `InvalidSize`.
5. Record `value_start = cursor.position()`.
6. `parse_property_value(cursor, &type_name, value_size as usize, names)`.
7. If `type_name != "BoolProperty"`: check `cursor.position() - value_start == value_size`; if not, seek to `value_start + value_size` (keeps stream synced on unknown types).
8. Push.

### Per-type tag layout (CRITICAL — these are the in-tag headers read BEFORE the value bytes):

- **BoolProperty**: value_size=0; a single byte follows array_index in the tag stream (part of tag, not value).
- **Int8Property, ByteProperty (size=1)**: 1 byte.
- **ByteProperty (size≠1)**: FName `enum_type`; if size==8 it's just the FName value; else FName `value`.
- **Int16/Int32/Int64/UInt16/UInt32/UInt64/Float/Double**: raw LE value.
- **StrProperty, TextProperty**: FString.
- **NameProperty**: FName.
- **ObjectProperty/InterfaceProperty/LazyObjectProperty**: `i32 index` → "None"/"Export[n]"/"Import[n]" string form.
- **SoftObjectProperty**: FString path + FString sub_path.
- **EnumProperty tag header**: FName enum_type + `u8 has_prop_guid`. Value: FName value.
- **StructProperty tag header**: FName struct_type + 16-byte GUID + u8 has_prop_guid. Then struct value (see native struct table).
- **ArrayProperty tag header**: FName inner_type + u8 has_prop_guid. Value: i32 count + elements (via `parse_array_element`).
- **MapProperty tag header**: FName key_type + FName value_type + u8 has_prop_guid. Value: i32 num_to_remove (0) + i32 count + entries `(key, value)`.
- **SetProperty tag header**: FName inner_type + u8 has_prop_guid. Value: i32 num_remove (0) + i32 count + elements.
- **Unknown**: read `value_size` bytes as `Opaque`.

### Native structs (`parse_struct_value`) — layout depends on `value_size` (LWC 5.0+ uses doubles):

| Struct | f32 layout (size) | f64 layout (size) |
|---|---|---|
| Vector, Vector_NetQuantize, Vector_NetQuantize100 | 3×f32 (12) | 3×f64 (24) |
| Vector2D | 2×f32 (8) | 2×f64 (16) |
| Vector4, Vector4f, Vector4d | 4×f32 (16) | 4×f64 (32) |
| Rotator | Pitch/Yaw/Roll f32 (12) | f64 (24) |
| Quat, Quat4f, Quat4d | 4×f32 (16) | 4×f64 (32) |
| LinearColor | 4×f32 R,G,B,A always | — |
| Color | 4×u8 in **B,G,R,A** order | — |
| IntPoint | 2×i32 | — |
| Guid | 16 bytes (stored as hex string) | — |
| DateTime, Timespan | i64 ticks | — |
| SoftObjectPath, SoftClassPath, StringAssetReference, StringClassReference | FString path | — |
| GameplayTag | FName TagName | — |
| GameplayTagContainer | i32 count + N×FName | — |
| Transform | recursive tagged property stream | — |
| Unknown | recursive tagged property stream (bounded by value_size) | — |

`parse_tagged_properties_from_cursor(cursor, names, value_size)` carves a slice `[pos, pos+value_size)`, calls `parse_tagged_properties` on it, seeks cursor past it regardless of result.

### `parse_array_element(inner_type)` — reads a single element. Scalars read their raw value (no tag header). StructProperty: read a nested tagged property stream until "None" terminator (sizes unknown per element). Unknown: `Opaque` with empty data.

### Error recovery
`parse_property_value` wraps `parse_property_value_inner` — on error, returns `Opaque{type_name, remaining bytes up to value_size}`. Outer sync-seek pins stream at `value_start + value_size` on mismatch.

### Serialization (write path)

Mirrors parse. Key helpers:
- `write_fname(buf, name, &mut names)`: `split_fname_suffix` splits trailing `_N` (decimal) — if N parses, base is `name[..pos]`, stored number is `N+1`. Otherwise `(name, 0)`. Index looked up (or appended). Writes u32 index + u32 number.
- `write_fstring`: empty → i32=0. Else len = bytes+1 (null terminator), write i32 len, bytes, `\0`. **Never emits UTF-16 — lossy for non-ASCII.**
- `parse_object_ref`: inverse of object-ref formatting (`"Export[N]"`→N+1, `"Import[N]"`→-(N+1), `"None"`→0).

`serialize_tagged_properties(props, &mut names) -> Vec<u8>`: per property:
- BoolProperty: write 2×FName + i32 size=0 + i32 array_index + u8 bool. `continue`.
- Else: write 2×FName + placeholder i32 size + i32 array_index. Record `value_start`. Call `serialize_property_value`. Backpatch `(buf.len() - value_start) as i32` into placeholder.
Finally write FName "None" terminator.

`serialize_property_value` + `serialize_struct_value` + `serialize_array_element` mirror the parse tables exactly. Vector components serialized by **looking up field by name** in the struct's field vec (order-independent on input; but writes in fixed order). `is_double` detected from first field variant. Color always 4×u8 BGRA regardless of input order. Guid expects hex string in `Value` field; non-16-byte hex yields zero GUID.

## 12. FField Definition Parser (ffield.rs)

Purpose: scan an export's byte range for UE's `SerializeProperties()` output (UClass/UStruct property declarations). Used by diffs to name added/removed variables.

### `FieldDefinition`
```rust
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
```
`Display`: `{field_type}<{struct_type|inner_type|key,value}> {field_name}` with `[N]` if `array_dim > 1`. *Note*: `array_dim`, `property_flags`, and sub-types are currently hardcoded defaults — parser extracts type+name only.

### `parse_field_definitions(export_data, names, class_name) -> Option<Vec<FieldDefinition>>`
1. Reject if class_name not in `{Class, ScriptStruct, *GeneratedClass, BlueprintGeneratedClass, WidgetBlueprintGeneratedClass, AnimBlueprintGeneratedClass}`.
2. Scan full buffer: for each offset `o`, treat `data[o..o+4]` as `PropertyCount`. Valid range `[1, 500]`. Check that `data[o+4..o+8]` is a name index pointing to a known field type (see `KNOWN_FIELD_TYPES`).
3. If valid, attempt `try_parse_fields_at(data, o, count, names)` — loops `count` times using `find_next_field_type` to advance to the next known FField type FName (must have `name_num == 0`). After each find, the property's name FName is at `type_pos + 8`. Skip forward by `MIN_FIELD_SKIP = 53` bytes (FField min struct) before searching next.
4. Call `filter_inner_types` — removes entries sharing the same `field_name` as the directly preceding container (Array/Map/Set/Enum).
5. Return the set with the most fields across all scan starts.

`KNOWN_FIELD_TYPES` (30 entries): see source. Must match FField classname registry for UE 4.25+ (unified FField hierarchy).

### Blueprint variable scanning (in structured.rs)
`scan_blueprint_variables(data, names)`: locates FName byte-patterns "VarName" + "NameProperty" in FBPVariableDescription structs. Bytes layout: 16 bytes of two FNames + 9 bytes header (size i32, array_index i32, has_property_guid u8), then FName value at offset+25. `find_pin_category_near` scans 200 bytes forward for "PinCategory" FName to recover the declared variable type (int→int32, real/float→float, etc. via `pin_category_to_type`).

## 13. StructuredAsset (structured.rs)

Highest-level API for diff tools. Combines everything above into a self-contained, serde-friendly tree.

```rust
pub struct StructuredAsset {
    pub engine_version: String,        // "{major}.{minor}.{patch}"
    pub package_flags: u32,
    pub names: Vec<String>,
    pub imports: Vec<ImportInfo>,
    pub exports: Vec<ExportInfo>,
    pub parse_warnings: Vec<String>,
}

pub struct ImportInfo { index, class_package, class_name, object_name, outer_name: Option<String> }

pub struct ExportInfo {
    index, object_name, class_name, serial_size: i64, serial_offset: i64,
    outer_name: Option<String>,
    properties: Option<Vec<TaggedProperty>>,
    field_definitions: Option<Vec<FieldDefinition>>,
    trailing_data_size: usize,
}

pub enum StructuredParseError { HeaderParseFailed(String), UnversionedProperties }
```

### `parse_structured(data)` → `parse_structured_with_uexp(data, None)`

### `parse_structured_with_uexp(header_data, uexp_data: Option<&[u8]>)`
1. `AssetHeader::new(Cursor::new(header_data))`; map err → `HeaderParseFailed`.
2. Detect cooked: `package_flags & UnversionedProperties != 0`. Cooked assets skip property parsing (UE class schemas needed), still emit export metadata. Add warning.
3. Build `file_data`: if `uexp_data` provided and non-empty, concat `header_data + uexp_data`. UE splits export payloads into the `.uexp` sidecar; export `serial_offset` is relative to the (notionally concatenated) file. Else use header_data directly.
4. For each import: resolve class_package/class_name/object_name via `header.resolve_name`; resolve outer_name via `imp.outer()` → look up in imports/exports as appropriate.
5. For each export:
   - Resolve object_name and class_name (class_index may point to import or export).
   - Resolve outer_name.
   - Cooked: skip property parsing; record `trailing_data_size = serial_size`.
   - Else: `parse_export_properties(file_data, export, names, warnings)` — determine prop range:
     - If `script_serialization_start_offset >= 0 && end > start`: `[serial_offset + start, serial_offset + end]` within bounds.
     - Else: `[serial_offset, serial_offset + serial_size]`.
     - If range exceeds `file_data.len()` → `(None, 0)` (property bytes live in absent `.uexp`).
     - Call `property::parse_tagged_properties`; compute `trailing = serial_size - (prop_end - serial_offset)`.
     - On error: warning + `(None, serial_size)`.
   - Also attempt `ffield::parse_field_definitions` over the full export slice `[serial_offset, +serial_size]`.

## 14. CLI (main.rs)

`uasset` binary (feature `commandline-tool`). Subcommands via `structopt`:
- `benchmark <paths...>` — time loading every asset.
- `dump <paths...>` — print every `AssetHeader` field.
- `validate <paths...> --mode All|HasEngineVersion,...` — run validations. Perforce integration via `--perforce-changelist`.
- `list-imports <paths...> [--skip-code-imports]` — uses `package_import_iter`.
- `list-object-types <paths...>` — filters exports by flags (Public/Standalone via `ObjectFlags`).
- `dump-thumbnail-info <paths...>` — uses `thumbnail_iter`.

`recursively_walk_uassets` via `walkdir`, filters to `.uasset`/`.umap` extensions (skipping dotfiles).

## 15. Tests

- `tests/basic_parsing.rs` — `loading_asset` and `upgrading_asset` across all UE versions via `rstest_reuse::apply(all_versions)`; validates `file_version`, `file_version_ue5`, package source monotonicity, presence of known asset paths in name table.
- `tests/asset_references.rs` — imports resolution smoke tests.
- `test_utilities` crate: exposes `UnrealVersionInfo` struct, `all_versions` fixture template, `UnrealVersion::get_asset_base_path()` → `assets/UEXYZ/`.
- Test assets live under `assets/UE4XX/SimpleRefs/...`.

## 16. Rebuild Order (Recommended)

1. `error.rs` (trivial) → `enums.rs` (just data).
2. `archive.rs` — needs `SerializedObjectVersion`/`SerializedFlags` traits live here.
3. `serialization.rs` traits + `serialization/implementations.rs` — blanket impls first, then `UnrealString/NameEntryWithHash/Array/Guid/CustomVersion/Generation/CompressedChunk/EngineVersion/NameReference/ObjectExport/ClassImport/ThumbnailInfo`.
4. `lib.rs` — top-level structs + `AssetHeader::new` (follow §8 order exactly).
5. `property.rs` — parse path first, then writer.
6. `ffield.rs` — independent of parse order.
7. `structured.rs` — uses everything above.
8. `main.rs` CLI (optional for minimal build).

## 17. Known Limitations / Watchpoints

- `find_name` does not handle `_N` suffixes (TODO in source).
- `write_fstring` never emits UTF-16 — non-ASCII input becomes invalid UE data.
- `ffield::parse_field_definitions` is a byte-pattern scanner, not a structural parser — `array_dim`/`property_flags`/sub-types are stubs.
- `StructProperty` inside arrays (`parse_array_element`) assumes tagged-property stream — native structs nested in arrays (e.g. Vector in TArray<FVector>) are NOT handled specially.
- Cooked assets (UnversionedProperties flag) cannot be property-parsed without UE schema reflection — structured parser returns header-only.
- `Color` field order is **B,G,R,A** on wire (not RGBA).
- Object reference serialization uses string form `"Export[n]"/"Import[n]"` — round-trip depends on `parse_object_ref`.
- `file_version_ue5` forward-compat: unknown UE5 versions collapse to `PACKAGE_SAVED_HASH` (1016). May miss future header fields.
- Version enums: every integer from 214..=522 must be a named variant for `FromPrimitive` — gaps cause spurious `UnsupportedUE4Version`.
- `legacy_version` accepted range: `[-9, -5]`. `-9` = UE5.6+ saved-hash format. `-8` = UE5 base. `-7` = UE4 (texture_allocations present). `-6/-5` = older UE4.
