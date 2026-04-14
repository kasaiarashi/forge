//! `Archive<R>`: tracks the file-version state pulled from a uasset's leading
//! summary so downstream parsers can gate field reads on engine version.

use crate::{enums::ObjectVersionUE5, Error, ObjectVersion, Result};
use binread::BinReaderExt;
use num_traits::FromPrimitive;
use std::io::{Read, Seek};

/// First u32 of every uasset; used both as a sentinel and to decide endianness.
const PACKAGE_FILE_MAGIC: u32 = 0x9E2A83C1;

/// Distinguishes the two different layouts UE used for the custom-version table.
pub enum CustomVersionSerializationFormat {
    /// Pre-optimized layout — each entry carries its own GUID + friendly name.
    Guids,
    /// Compact layout — fixed-size 20-byte records.
    Optimized,
}

/// Versioning state for a uasset together with the underlying byte source.
#[derive(Debug)]
pub struct Archive<R> {
    pub reader: R,
    /// `FileVersionUE4` — the UE4 object-graph version (always present).
    pub file_version: ObjectVersion,
    /// `FileVersionUE5` — `Some` only for assets saved by UE5.
    pub file_version_ue5: Option<ObjectVersionUE5>,
    /// `FileVersionLicenseeUE4` — licensee-specific version stamp.
    pub file_licensee_version: i32,
    /// The serialization "legacy" version (negative integer); selects layout
    /// branches inside `FPackageFileSummary`.
    pub legacy_version: i32,
    /// Mirrors `package_flags` not having `FilterEditorOnly` set; populated by
    /// the header parser after it has read the package flags.
    pub with_editoronly_data: bool,
}

impl<R> Archive<R>
where
    R: Seek + Read,
{
    pub fn new(mut reader: R) -> Result<Self> {
        let magic: u32 = reader.read_le()?;
        if magic != PACKAGE_FILE_MAGIC {
            return Err(Error::InvalidFile);
        }

        // See `operator<<(FStructuredArchive::FSlot, FPackageFileSummary&)` in
        // Engine/Source/Runtime/CoreUObject/Private/UObject/PackageFileSummary.cpp.
        let legacy_version: i32 = reader.read_le()?;
        if !(-9..=-5).contains(&legacy_version) {
            return Err(Error::UnsupportedVersion(legacy_version));
        }

        // LegacyUE3Version — discarded.
        let _ue3_version: i32 = reader.read_le()?;

        let raw_ue4_version: i32 = reader.read_le()?;

        let raw_ue5_version: i32 = if legacy_version <= -8 {
            reader.read_le()?
        } else {
            0
        };

        let file_licensee_version: i32 = reader.read_le()?;

        if raw_ue4_version == 0 && raw_ue5_version == 0 && file_licensee_version == 0 {
            return Err(Error::UnversionedAsset);
        }

        if raw_ue4_version == 0 {
            return Err(Error::UnsupportedUE4Version(raw_ue4_version));
        }
        let file_version = ObjectVersion::from_i32(raw_ue4_version)
            .ok_or(Error::UnsupportedUE4Version(raw_ue4_version))?;

        // For UE5 versions we treat unknown future values as `PACKAGE_SAVED_HASH`
        // — UE only ever appends fields, so the older known layout is a safe upper bound.
        let file_version_ue5 = if raw_ue5_version != 0 {
            Some(
                ObjectVersionUE5::from_i32(raw_ue5_version)
                    .unwrap_or(ObjectVersionUE5::PACKAGE_SAVED_HASH),
            )
        } else {
            None
        };

        Ok(Self {
            reader,
            file_version,
            file_version_ue5,
            file_licensee_version,
            legacy_version,
            with_editoronly_data: false,
        })
    }

    pub fn reader(&mut self) -> &mut R {
        &mut self.reader
    }

    pub fn custom_version_serialization_format(&self) -> CustomVersionSerializationFormat {
        if self.legacy_version < -5 {
            CustomVersionSerializationFormat::Optimized
        } else {
            CustomVersionSerializationFormat::Guids
        }
    }
}

/// Mirror of UE's `IsFilterEditorOnly()` — exposes whether editor-only fields
/// are present in the stream.
pub trait SerializedFlags {
    fn serialized_with_editoronly_data(&self) -> bool;
}

impl<R> SerializedFlags for Archive<R> {
    fn serialized_with_editoronly_data(&self) -> bool {
        self.with_editoronly_data
    }
}

/// Generic "is this version active" gate, used by parsers to branch on a
/// specific UE4 / UE5 version threshold.
pub trait SerializedObjectVersion<T> {
    fn serialized_with(&self, version: T) -> bool;

    fn serialized_without(&self, version: T) -> bool {
        !self.serialized_with(version)
    }
}

impl<R> SerializedObjectVersion<ObjectVersion> for Archive<R> {
    fn serialized_with(&self, version: ObjectVersion) -> bool {
        self.file_version >= version
    }
}

impl<R> SerializedObjectVersion<ObjectVersionUE5> for Archive<R> {
    fn serialized_with(&self, version: ObjectVersionUE5) -> bool {
        match self.file_version_ue5 {
            Some(v) => v >= version,
            None => false,
        }
    }
}

impl<R> Read for Archive<R>
where
    R: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

impl<R> Seek for Archive<R>
where
    R: Seek,
{
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.reader.seek(pos)
    }
}
