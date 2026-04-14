//! Concrete `Parseable`/`Skippable` implementations for the building-block
//! types that appear inside `FPackageFileSummary` and friends.

use binread::{BinRead, BinReaderExt};
use bit_field::BitField;
use std::{
    io::{Read, Seek, SeekFrom},
    marker::PhantomData,
    mem::size_of,
    num::NonZeroU32,
};

use crate::{
    archive::{SerializedFlags, SerializedObjectVersion},
    serialization::{
        ArrayStreamInfo, Deferrable, Parseable, ReadInfo, SingleItemStreamInfo, Skippable,
        StreamInfo,
    },
    AssetHeader, Error, NameReference, ObjectExport, ObjectImport, ObjectVersion,
    ObjectVersionUE5, Result, ThumbnailInfo,
};

// Blanket impls for any `BinRead` primitive (i32, u64, f32, ...): a single
// little-endian read or a sized seek.

impl<T> Deferrable for T
where
    T: BinRead,
{
    type StreamInfoType = SingleItemStreamInfo;
}

impl<T> Skippable for T
where
    T: BinRead + Sized,
{
    fn seek_past_with_info<R>(reader: &mut R, info: &Self::StreamInfoType) -> Result<()>
    where
        R: Seek + Read,
    {
        reader.seek(SeekFrom::Start(info.offset + size_of::<T>() as u64))?;
        Ok(())
    }
}

impl<T> Parseable for T
where
    T: BinRead,
{
    type ParsedType = T;

    fn parse_with_info_seekless<R>(
        reader: &mut R,
        _info: &<Self::StreamInfoType as StreamInfo>::ReadInfoType,
    ) -> Result<Self::ParsedType>
    where
        R: Seek + Read,
    {
        Ok(reader.read_le()?)
    }
}

// --- FString helpers ---------------------------------------------------------

/// Skip past an `FString` without materializing it.
fn skip_string<R>(reader: &mut R) -> Result<()>
where
    R: Seek + Read,
{
    let raw_len: i32 = reader.read_le()?;
    // Negative length signals a UCS-2 string (2 bytes per code unit).
    let (chars, bytes_per_char) = if raw_len < 0 {
        (-raw_len, 2i64)
    } else {
        (raw_len, 1i64)
    };
    reader.seek(SeekFrom::Current(chars as i64 * bytes_per_char))?;
    Ok(())
}

/// Decode an `FString`. UCS-2 input is hand-converted to UTF-8 a code-unit at a
/// time using `bit_field` for the multi-byte branches; null terminator is
/// always stripped.
fn parse_string<R>(reader: &mut R) -> Result<String>
where
    R: Seek + Read,
{
    let raw_len: i32 = reader.read_le()?;
    if raw_len == 0 {
        return Ok(String::new());
    }

    if raw_len < 0 {
        // UCS-2 (BMP only).
        let total = (-raw_len) as usize;
        let payload_chars = total - 1; // drop trailing NUL
        let mut out: Vec<u8> = Vec::with_capacity(payload_chars * 3);

        for _ in 0..payload_chars {
            let cu: u16 = reader.read_le()?;
            match cu {
                0x0000..=0x007F => out.push(cu as u8),
                0x0080..=0x07FF => {
                    let b0 = 0b1100_0000u8 | cu.get_bits(6..11) as u8;
                    let b1 = 0b1000_0000u8 | cu.get_bits(0..6) as u8;
                    out.push(b0);
                    out.push(b1);
                }
                _ => {
                    let b0 = 0b1110_0000u8 | cu.get_bits(12..16) as u8;
                    let b1 = 0b1000_0000u8 | cu.get_bits(6..12) as u8;
                    let b2 = 0b1000_0000u8 | cu.get_bits(0..6) as u8;
                    out.push(b0);
                    out.push(b1);
                    out.push(b2);
                }
            }
        }
        // Skip the trailing NUL u16.
        reader.seek(SeekFrom::Current(2))?;

        out.shrink_to_fit();
        String::from_utf8(out).map_err(Error::InvalidString)
    } else {
        let payload = (raw_len - 1) as usize;
        let mut buf = vec![0u8; payload];
        reader.read_exact(&mut buf)?;
        // Skip the trailing NUL byte.
        reader.seek(SeekFrom::Current(1))?;
        String::from_utf8(buf).map_err(Error::InvalidString)
    }
}

// --- UnrealString ------------------------------------------------------------

#[derive(Debug)]
pub struct UnrealString {}

impl Deferrable for UnrealString {
    type StreamInfoType = SingleItemStreamInfo;
}

impl Skippable for UnrealString {
    fn seek_past_with_info<R>(reader: &mut R, info: &Self::StreamInfoType) -> Result<()>
    where
        R: Seek + Read,
    {
        reader.seek(SeekFrom::Start(info.offset))?;
        skip_string(reader)
    }
}

impl Parseable for UnrealString {
    type ParsedType = String;

    fn parse_with_info_seekless<R>(
        reader: &mut R,
        _info: &<Self::StreamInfoType as StreamInfo>::ReadInfoType,
    ) -> Result<Self::ParsedType>
    where
        R: Seek + Read,
    {
        parse_string(reader)
    }
}

// --- UnrealNameEntryWithHash -------------------------------------------------

#[derive(Debug)]
pub struct UnrealNameEntryWithHash {}

impl Deferrable for UnrealNameEntryWithHash {
    type StreamInfoType = SingleItemStreamInfo;
}

impl Skippable for UnrealNameEntryWithHash {
    fn seek_past_with_info<R>(reader: &mut R, info: &Self::StreamInfoType) -> Result<()>
    where
        R: Seek + Read,
    {
        reader.seek(SeekFrom::Start(info.offset))?;
        skip_string(reader)?;
        // Two u16 hashes (NonCasePreserving + CasePreserving) follow the name.
        reader.seek(SeekFrom::Current(size_of::<[u16; 2]>() as i64))?;
        Ok(())
    }
}

impl Parseable for UnrealNameEntryWithHash {
    type ParsedType = String;

    fn parse_with_info_seekless<R>(
        reader: &mut R,
        _info: &<Self::StreamInfoType as StreamInfo>::ReadInfoType,
    ) -> Result<Self::ParsedType>
    where
        R: Seek + Read,
    {
        let s = parse_string(reader)?;
        let _hash: u32 = reader.read_le()?;
        Ok(s)
    }
}

// --- UnrealArray<E> ---------------------------------------------------------

#[derive(Debug)]
pub struct UnrealArray<ElementType>
where
    ElementType: Sized,
{
    _items: Vec<ElementType>,
}

impl<E> Deferrable for UnrealArray<E> {
    type StreamInfoType = ArrayStreamInfo;
}

impl<E, ESI> Skippable for UnrealArray<E>
where
    E: Skippable<StreamInfoType = ESI>,
    ESI: StreamInfo,
{
    fn seek_past_with_info<R>(reader: &mut R, info: &Self::StreamInfoType) -> Result<()>
    where
        R: Seek + Read,
    {
        reader.seek(SeekFrom::Start(info.offset))?;
        let mut remaining = info.count;
        while remaining > 0 {
            let elem_info = ESI::from_current_position(reader)?;
            E::seek_past_with_info(reader, &elem_info)?;
            remaining -= 1;
        }
        Ok(())
    }
}

impl<E, ESI> Parseable for UnrealArray<E>
where
    E: Parseable<StreamInfoType = ESI>,
    ESI: StreamInfo,
{
    type ParsedType = Vec<E::ParsedType>;

    fn parse_with_info_seekless<R>(
        reader: &mut R,
        info: &<Self::StreamInfoType as StreamInfo>::ReadInfoType,
    ) -> Result<Self::ParsedType>
    where
        R: Seek
            + Read
            + SerializedObjectVersion<ObjectVersion>
            + SerializedObjectVersion<ObjectVersionUE5>
            + SerializedFlags,
    {
        let n = info.count as usize;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let read_info = ESI::ReadInfoType::from_current_position(reader)?;
            out.push(E::parse_with_info_seekless(reader, &read_info)?);
        }
        Ok(out)
    }
}

// --- UnrealArrayIterator -----------------------------------------------------

#[derive(Debug)]
pub struct UnrealArrayIterator<'a, ElementType, R>
where
    ElementType: Parseable,
{
    package: &'a mut AssetHeader<R>,
    stream_info: ArrayStreamInfo,
    next_index: u64,
    _phantom: PhantomData<ElementType>,
}

impl<'a, E, ESI, R> UnrealArrayIterator<'a, E, R>
where
    E: Parseable<StreamInfoType = ESI>,
    ESI: StreamInfo,
    R: Seek + Read,
{
    pub fn new(package: &'a mut AssetHeader<R>, stream_info: ArrayStreamInfo) -> Result<Self> {
        package.archive.seek(SeekFrom::Start(stream_info.offset))?;
        Ok(Self {
            package,
            stream_info,
            next_index: 0,
            _phantom: PhantomData,
        })
    }
}

impl<'a, E, ESI, R> Iterator for UnrealArrayIterator<'a, E, R>
where
    E: Parseable<StreamInfoType = ESI>,
    ESI: StreamInfo,
    R: Seek + Read,
{
    type Item = Result<E::ParsedType>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_index >= self.stream_info.count {
            return None;
        }
        self.next_index += 1;
        let result = ESI::ReadInfoType::from_current_position(&mut self.package.archive)
            .and_then(|ri| E::parse_with_info_seekless(&mut self.package.archive, &ri));
        Some(result)
    }
}

// --- Fixed-size skippable structs -------------------------------------------

const GUID_SIZE: u64 = 16;

pub struct UnrealGuid {}

impl Deferrable for UnrealGuid {
    type StreamInfoType = SingleItemStreamInfo;
}

impl Skippable for UnrealGuid {
    fn seek_past_with_info<R>(reader: &mut R, info: &Self::StreamInfoType) -> Result<()>
    where
        R: Seek + Read,
    {
        reader.seek(SeekFrom::Start(info.offset + GUID_SIZE))?;
        Ok(())
    }
}

const CUSTOM_VERSION_SIZE: u64 = 20;

pub struct UnrealCustomVersion {}

impl Deferrable for UnrealCustomVersion {
    type StreamInfoType = SingleItemStreamInfo;
}

impl Skippable for UnrealCustomVersion {
    fn seek_past_with_info<R>(reader: &mut R, info: &Self::StreamInfoType) -> Result<()>
    where
        R: Seek + Read,
    {
        reader.seek(SeekFrom::Start(info.offset + CUSTOM_VERSION_SIZE))?;
        Ok(())
    }
}

const GUID_CUSTOM_VERSION_PREFIX_SIZE: u64 = 20;

pub struct UnrealGuidCustomVersion {}

impl Deferrable for UnrealGuidCustomVersion {
    type StreamInfoType = SingleItemStreamInfo;
}

impl Skippable for UnrealGuidCustomVersion {
    fn seek_past_with_info<R>(reader: &mut R, info: &Self::StreamInfoType) -> Result<()>
    where
        R: Seek + Read,
    {
        reader.seek(SeekFrom::Start(
            info.offset + GUID_CUSTOM_VERSION_PREFIX_SIZE,
        ))?;
        UnrealString::seek_past(reader)?;
        Ok(())
    }
}

const GENERATION_INFO_SIZE: u64 = 8;

pub struct UnrealGenerationInfo {}

impl Deferrable for UnrealGenerationInfo {
    type StreamInfoType = SingleItemStreamInfo;
}

impl Skippable for UnrealGenerationInfo {
    fn seek_past_with_info<R>(reader: &mut R, info: &Self::StreamInfoType) -> Result<()>
    where
        R: Seek + Read,
    {
        reader.seek(SeekFrom::Start(info.offset + GENERATION_INFO_SIZE))?;
        Ok(())
    }
}

const COMPRESSED_CHUNK_SIZE: u64 = 16;

pub struct UnrealCompressedChunk {}

impl Deferrable for UnrealCompressedChunk {
    type StreamInfoType = SingleItemStreamInfo;
}

impl Skippable for UnrealCompressedChunk {
    fn seek_past_with_info<R>(reader: &mut R, info: &Self::StreamInfoType) -> Result<()>
    where
        R: Seek + Read,
    {
        reader.seek(SeekFrom::Start(info.offset + COMPRESSED_CHUNK_SIZE))?;
        Ok(())
    }
}

// --- UnrealEngineVersion -----------------------------------------------------

/// Decoded `FEngineVersion` — `changelist` is masked to 31 bits and the high
/// bit is exposed as `is_licensee_version`.
#[derive(Clone, Debug)]
pub struct UnrealEngineVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
    pub changelist: u32,
    pub is_licensee_version: bool,
    pub branch_name: String,
}

impl UnrealEngineVersion {
    pub const LICENSEE_BIT_MASK: u32 = 0x8000_0000;
    pub const CHANGELIST_MASK: u32 = 0x7fff_ffff;

    pub fn empty() -> Self {
        Self {
            major: 0,
            minor: 0,
            patch: 0,
            changelist: 0,
            is_licensee_version: false,
            branch_name: String::new(),
        }
    }

    /// Reconstruct from a bare changelist (used for pre-`ENGINE_VERSION_OBJECT`
    /// assets that only stored the CL).
    pub fn from_changelist(changelist: u32) -> Self {
        Self {
            major: 4,
            minor: 0,
            patch: 0,
            changelist: changelist & Self::CHANGELIST_MASK,
            is_licensee_version: (changelist & Self::LICENSEE_BIT_MASK) != 0,
            branch_name: String::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.changelist == 0 && !self.is_licensee_version
    }
}

impl Deferrable for UnrealEngineVersion {
    type StreamInfoType = SingleItemStreamInfo;
}

impl Parseable for UnrealEngineVersion {
    type ParsedType = UnrealEngineVersion;

    fn parse_with_info_seekless<R>(
        reader: &mut R,
        _info: &<Self::StreamInfoType as StreamInfo>::ReadInfoType,
    ) -> Result<Self::ParsedType>
    where
        R: Seek
            + Read
            + SerializedObjectVersion<ObjectVersion>
            + SerializedObjectVersion<ObjectVersionUE5>
            + SerializedFlags,
    {
        let major: u16 = reader.read_le()?;
        let minor: u16 = reader.read_le()?;
        let patch: u16 = reader.read_le()?;
        let raw_cl: u32 = reader.read_le()?;
        let branch_name = UnrealString::parse_inline(reader)?;

        let mut v = Self::from_changelist(raw_cl);
        v.major = major;
        v.minor = minor;
        v.patch = patch;
        v.branch_name = branch_name;
        Ok(v)
    }
}

// --- UnrealNameReference -----------------------------------------------------

#[derive(Debug)]
pub struct UnrealNameReference {}

impl Deferrable for UnrealNameReference {
    type StreamInfoType = SingleItemStreamInfo;
}

impl Parseable for UnrealNameReference {
    type ParsedType = NameReference;

    fn parse_with_info_seekless<R>(
        reader: &mut R,
        _info: &<Self::StreamInfoType as StreamInfo>::ReadInfoType,
    ) -> Result<Self::ParsedType>
    where
        R: Seek + Read,
    {
        let index: u32 = reader.read_le()?;
        let raw_number: u32 = reader.read_le()?;
        Ok(NameReference {
            index,
            number: NonZeroU32::new(raw_number),
        })
    }
}

// --- UnrealObjectExport ------------------------------------------------------

#[derive(Debug)]
pub struct UnrealObjectExport {}

impl Deferrable for UnrealObjectExport {
    type StreamInfoType = SingleItemStreamInfo;
}

impl Parseable for UnrealObjectExport {
    type ParsedType = ObjectExport;

    fn parse_with_info_seekless<R>(
        reader: &mut R,
        _info: &<Self::StreamInfoType as StreamInfo>::ReadInfoType,
    ) -> Result<Self::ParsedType>
    where
        R: Seek
            + Read
            + SerializedObjectVersion<ObjectVersion>
            + SerializedObjectVersion<ObjectVersionUE5>
            + SerializedFlags,
    {
        let class_index: i32 = reader.read_le()?;
        let super_index: i32 = reader.read_le()?;

        let template_index: i32 =
            if reader.serialized_with(ObjectVersion::VER_UE4_TemplateIndex_IN_COOKED_EXPORTS) {
                reader.read_le()?
            } else {
                0
            };

        let outer_index: i32 = reader.read_le()?;
        let object_name = UnrealNameReference::parse_inline(reader)?;
        let object_flags: u32 = reader.read_le()?;

        let (serial_size, serial_offset): (i64, i64) =
            if reader.serialized_with(ObjectVersion::VER_UE4_64BIT_EXPORTMAP_SERIALSIZES) {
                (reader.read_le()?, reader.read_le()?)
            } else {
                let s: i32 = reader.read_le()?;
                let o: i32 = reader.read_le()?;
                (s as i64, o as i64)
            };

        let forced_export = reader.read_le::<u32>()? != 0;
        let not_for_client = reader.read_le::<u32>()? != 0;
        let not_for_server = reader.read_le::<u32>()? != 0;

        if !reader.serialized_with(ObjectVersionUE5::REMOVE_OBJECT_EXPORT_PACKAGE_GUID) {
            // Per-export package GUID, no longer carried in newer UE5 exports.
            UnrealGuid::seek_past(reader)?;
        }

        let is_inherited_instance =
            if reader.serialized_with(ObjectVersionUE5::TRACK_OBJECT_EXPORT_IS_INHERITED) {
                reader.read_le::<u32>()? != 0
            } else {
                false
            };

        let package_flags: u32 = reader.read_le()?;

        let not_always_loaded_for_editor_game =
            if reader.serialized_with(ObjectVersion::VER_UE4_LOAD_FOR_EDITOR_GAME) {
                reader.read_le::<u32>()? != 0
            } else {
                true
            };

        let is_asset =
            if reader.serialized_with(ObjectVersion::VER_UE4_COOKED_ASSETS_IN_EDITOR_SUPPORT) {
                reader.read_le::<u32>()? != 0
            } else {
                false
            };

        let generate_public_hash =
            if reader.serialized_with(ObjectVersionUE5::OPTIONAL_RESOURCES) {
                reader.read_le::<u32>()? != 0
            } else {
                false
            };

        let (
            first_export_dependency,
            serialization_before_serialization_dependencies,
            create_before_serialization_dependencies,
            serialization_before_create_dependencies,
            create_before_create_dependencies,
        ) = if reader
            .serialized_with(ObjectVersion::VER_UE4_PRELOAD_DEPENDENCIES_IN_COOKED_EXPORTS)
        {
            (
                reader.read_le()?,
                reader.read_le()?,
                reader.read_le()?,
                reader.read_le()?,
                reader.read_le()?,
            )
        } else {
            (-1i32, -1i32, -1i32, -1i32, -1i32)
        };

        let (script_serialization_start_offset, script_serialization_end_offset): (i64, i64) =
            if reader.serialized_with(ObjectVersionUE5::SCRIPT_SERIALIZATION_OFFSET) {
                (reader.read_le()?, reader.read_le()?)
            } else {
                (0, 0)
            };

        Ok(ObjectExport {
            outer_index,
            object_name,
            class_index,
            super_index,
            template_index,
            object_flags,
            serial_size,
            serial_offset,
            script_serialization_start_offset,
            script_serialization_end_offset,
            forced_export,
            not_for_client,
            not_for_server,
            not_always_loaded_for_editor_game,
            is_asset,
            is_inherited_instance,
            generate_public_hash,
            package_flags,
            first_export_dependency,
            serialization_before_serialization_dependencies,
            create_before_serialization_dependencies,
            serialization_before_create_dependencies,
            create_before_create_dependencies,
        })
    }
}

// --- UnrealClassImport -------------------------------------------------------

#[derive(Debug)]
pub struct UnrealClassImport {}

impl Deferrable for UnrealClassImport {
    type StreamInfoType = SingleItemStreamInfo;
}

impl Parseable for UnrealClassImport {
    type ParsedType = ObjectImport;

    fn parse_with_info_seekless<R>(
        reader: &mut R,
        _info: &<Self::StreamInfoType as StreamInfo>::ReadInfoType,
    ) -> Result<Self::ParsedType>
    where
        R: Seek
            + Read
            + SerializedObjectVersion<ObjectVersion>
            + SerializedObjectVersion<ObjectVersionUE5>
            + SerializedFlags,
    {
        let class_package = UnrealNameReference::parse_inline(reader)?;
        let class_name = UnrealNameReference::parse_inline(reader)?;
        let outer_index: i32 = reader.read_le()?;
        let object_name = UnrealNameReference::parse_inline(reader)?;

        let package_name = if reader
            .serialized_with(ObjectVersion::VER_UE4_NON_OUTER_PACKAGE_IMPORT)
            && reader.serialized_with_editoronly_data()
        {
            Some(UnrealNameReference::parse_inline(reader)?)
        } else {
            None
        };

        let import_optional = if reader.serialized_with(ObjectVersionUE5::OPTIONAL_RESOURCES) {
            reader.read_le::<u32>()? != 0
        } else {
            false
        };

        Ok(ObjectImport {
            outer_index,
            object_name,
            class_package,
            class_name,
            package_name,
            import_optional,
        })
    }
}

// --- UnrealThumbnailInfo -----------------------------------------------------

#[derive(Debug)]
pub struct UnrealThumbnailInfo {}

impl Deferrable for UnrealThumbnailInfo {
    type StreamInfoType = SingleItemStreamInfo;
}

impl Parseable for UnrealThumbnailInfo {
    type ParsedType = ThumbnailInfo;

    fn parse_with_info_seekless<R>(
        reader: &mut R,
        _info: &<Self::StreamInfoType as StreamInfo>::ReadInfoType,
    ) -> Result<Self::ParsedType>
    where
        R: Seek
            + Read
            + SerializedObjectVersion<ObjectVersion>
            + SerializedObjectVersion<ObjectVersionUE5>
            + SerializedFlags,
    {
        let object_class_name = UnrealString::parse_inline(reader)?;
        let object_path_without_package_name = UnrealString::parse_inline(reader)?;
        let file_offset: i32 = reader.read_le()?;
        Ok(ThumbnailInfo {
            object_class_name,
            object_path_without_package_name,
            file_offset,
        })
    }
}
