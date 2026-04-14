//! Forge's Unreal Engine `.uasset` / `.umap` parser.
//!
//! Provides:
//! - [`AssetHeader`] — the entire `FPackageFileSummary` plus name table,
//!   import map, and export map.
//! - [`property`] — tagged property tree parser/writer.
//! - [`ffield`] — pattern-based field-definition scanner for class/struct
//!   exports.
//! - [`structured`] — combined view used by Forge's diff and chunking layers.

mod archive;
pub mod enums;
mod error;
pub mod ffield;
pub mod property;
pub mod structured;
pub mod serialization;

use binread::BinReaderExt;
use std::{
    borrow::Cow,
    cmp::Ordering,
    io::{Read, Seek, SeekFrom},
    num::NonZeroU32,
};

use archive::SerializedObjectVersion;
use serialization::{
    ArrayStreamInfo, Parseable, Skippable, StreamInfo, UnrealArray, UnrealArrayIterator,
    UnrealClassImport, UnrealCompressedChunk, UnrealCustomVersion, UnrealEngineVersion,
    UnrealGenerationInfo, UnrealGuid, UnrealGuidCustomVersion, UnrealNameEntryWithHash,
    UnrealObjectExport, UnrealString, UnrealThumbnailInfo,
};

pub use archive::{Archive, CustomVersionSerializationFormat};
pub use enums::{ObjectVersion, ObjectVersionUE5, PackageFlags};
pub use error::{Error, InvalidNameIndexError, Result};

/// An index into [`AssetHeader::names`]. Resolve via [`AssetHeader::resolve_name`].
///
/// `number` is one-based: `Some(1)` is rendered as `_0`. Comparisons are only
/// meaningful between references obtained from the same asset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NameReference {
    pub index: u32,
    pub number: Option<NonZeroU32>,
}

/// Tagged reference to either an import, an export, or nothing.
#[derive(Debug)]
pub enum ObjectReference {
    None,
    Export { export_index: usize },
    Import { import_index: usize },
}

impl From<i32> for ObjectReference {
    fn from(raw: i32) -> Self {
        match raw.cmp(&0) {
            Ordering::Equal => ObjectReference::None,
            Ordering::Greater => ObjectReference::Export {
                export_index: (raw - 1) as usize,
            },
            Ordering::Less => ObjectReference::Import {
                import_index: -(raw + 1) as usize,
            },
        }
    }
}

/// Parsed `FObjectExport` row.
#[derive(Debug)]
pub struct ObjectExport {
    outer_index: i32,
    pub object_name: NameReference,

    class_index: i32,
    super_index: i32,
    template_index: i32,

    pub object_flags: u32,

    pub serial_size: i64,
    pub serial_offset: i64,

    pub script_serialization_start_offset: i64,
    pub script_serialization_end_offset: i64,

    pub forced_export: bool,
    pub not_for_client: bool,
    pub not_for_server: bool,

    pub not_always_loaded_for_editor_game: bool,

    pub is_asset: bool,
    pub is_inherited_instance: bool,
    pub generate_public_hash: bool,

    pub package_flags: u32,

    // Preload-dependency block; -1 means absent.
    pub first_export_dependency: i32,
    pub serialization_before_serialization_dependencies: i32,
    pub create_before_serialization_dependencies: i32,
    pub serialization_before_create_dependencies: i32,
    pub create_before_create_dependencies: i32,
}

impl ObjectExport {
    pub fn outer(&self) -> ObjectReference {
        ObjectReference::from(self.outer_index)
    }
    pub fn class(&self) -> ObjectReference {
        ObjectReference::from(self.class_index)
    }
    pub fn superclass(&self) -> ObjectReference {
        ObjectReference::from(self.super_index)
    }
    pub fn template(&self) -> ObjectReference {
        ObjectReference::from(self.template_index)
    }
}

/// Parsed `FObjectImport` row.
#[derive(Debug)]
pub struct ObjectImport {
    outer_index: i32,
    pub object_name: NameReference,
    pub class_package: NameReference,
    pub class_name: NameReference,
    pub package_name: Option<NameReference>,
    pub import_optional: bool,
}

impl ObjectImport {
    pub fn outer(&self) -> ObjectReference {
        ObjectReference::from(self.outer_index)
    }
}

/// Iterates the dependency package names listed in an asset's import map.
///
/// Filters to imports whose `class_name` is the literal `"Package"` and whose
/// `object_name` is not the engine sentinel `/Script/CoreUObject`.
pub struct ImportIterator<'a, R> {
    package: &'a AssetHeader<R>,
    next_index: usize,
    package_name_ref: NameReference,
    core_uobject_ref: Option<NameReference>,
}

impl<'a, R> ImportIterator<'a, R> {
    pub fn new(package: &'a AssetHeader<R>) -> Self {
        let package_name_ref = package.find_name("Package");
        let core_uobject_ref = package.find_name("/Script/CoreUObject");

        match package_name_ref {
            Some(pkg) => Self {
                package,
                next_index: 0,
                package_name_ref: pkg,
                core_uobject_ref,
            },
            // No "Package" name → nothing matches; arrange to terminate immediately.
            None => Self {
                package,
                next_index: package.imports.len(),
                package_name_ref: NameReference {
                    index: 0,
                    number: None,
                },
                core_uobject_ref,
            },
        }
    }
}

impl<'a, R> Iterator for ImportIterator<'a, R> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        while self.next_index < self.package.imports.len() {
            let imp = &self.package.imports[self.next_index];
            self.next_index += 1;

            let is_pkg = imp.class_name == self.package_name_ref;
            let not_core = self
                .core_uobject_ref
                .is_some_and(|n| imp.object_name != n);
            if is_pkg && not_core {
                if let Ok(name) = self.package.resolve_name(&imp.object_name) {
                    return Some(name.into_owned());
                }
            }
        }
        None
    }
}

/// Asset thumbnail metadata, as referenced by [`AssetHeader::thumbnail_table_offset`].
#[derive(Debug)]
pub struct ThumbnailInfo {
    pub object_class_name: String,
    pub object_path_without_package_name: String,
    pub file_offset: i32,
}

/// Loaded `FPackageFileSummary` plus the resolved name / import / export tables.
#[derive(Debug)]
pub struct AssetHeader<R> {
    pub archive: Archive<R>,
    pub total_header_size: i32,
    pub package_name: String,
    pub package_flags: u32,
    pub names: Vec<String>,
    pub soft_object_paths_count: i32,
    pub soft_object_paths_offset: i32,
    pub localization_id: Option<String>,
    pub gatherable_text_data_count: i32,
    pub gatherable_text_data_offset: i32,
    pub exports: Vec<ObjectExport>,
    pub imports: Vec<ObjectImport>,
    pub depends_offset: i32,
    pub soft_package_references_count: i32,
    pub soft_package_references_offset: i32,
    pub searchable_names_offset: Option<i32>,
    pub thumbnail_table_offset: i32,
    pub engine_version: UnrealEngineVersion,
    pub compatible_with_engine_version: UnrealEngineVersion,
    pub compression_flags: u32,
    pub package_source: u32,
    pub additional_packages_to_cook: Vec<String>,
    pub texture_allocations: Option<i32>,
    pub asset_registry_data_offset: i32,
    pub bulk_data_start_offset: i64,
    pub world_tile_info_data_offset: Option<i32>,
    pub chunk_ids: Vec<i32>,
    pub preload_dependency_count: i32,
    pub preload_dependency_offset: i32,
    pub names_referenced_from_export_data_count: i32,
    pub payload_toc_offset: i64,
    pub data_resource_offset: Option<i32>,
}

impl<R> AssetHeader<R>
where
    R: Seek + Read,
{
    /// Parse an entire asset summary from a little-endian stream.
    ///
    /// Field order follows §8 of the rebuild spec — every conditional read is
    /// version-gated and reordering them will desync the stream.
    pub fn new(reader: R) -> Result<Self> {
        let mut archive = Archive::new(reader)?;

        // UE5.6+ moved SavedHash + TotalHeaderSize ahead of the custom-version
        // table; older formats kept TotalHeaderSize after it.
        let new_format = archive
            .file_version_ue5
            .map(|v| v >= ObjectVersionUE5::PACKAGE_SAVED_HASH)
            .unwrap_or(false);

        let early_total_header_size: i32 = if new_format {
            // FIoHash SavedHash (20 bytes) — discarded.
            let mut hash = [0u8; 20];
            archive.read_exact(&mut hash)?;
            archive.read_le()?
        } else {
            0
        };

        // Custom version container (parsed-and-skipped — we don't surface them).
        let custom_versions_info = ArrayStreamInfo::from_current_position(&mut archive)?;
        match archive.custom_version_serialization_format() {
            CustomVersionSerializationFormat::Guids => {
                UnrealArray::<UnrealGuidCustomVersion>::seek_past_with_info(
                    &mut archive,
                    &custom_versions_info,
                )?;
            }
            CustomVersionSerializationFormat::Optimized => {
                UnrealArray::<UnrealCustomVersion>::seek_past_with_info(
                    &mut archive,
                    &custom_versions_info,
                )?;
            }
        }

        let total_header_size: i32 = if new_format {
            early_total_header_size
        } else {
            archive.read_le()?
        };

        let package_name = UnrealString::parse_inline(&mut archive)?;

        let package_flags: u32 = archive.read_le()?;
        let editor_only = (package_flags & PackageFlags::FilterEditorOnly as u32) == 0;
        archive.with_editoronly_data = editor_only;

        // Name table layout switched at VER_UE4_NAME_HASHES_SERIALIZED.
        let names = if archive.serialized_with(ObjectVersion::VER_UE4_NAME_HASHES_SERIALIZED) {
            UnrealArray::<UnrealNameEntryWithHash>::parse_indirect(&mut archive)?
        } else {
            UnrealArray::<UnrealString>::parse_indirect(&mut archive)?
        };

        let (soft_object_paths_count, soft_object_paths_offset) =
            if archive.serialized_with(ObjectVersionUE5::ADD_SOFTOBJECTPATH_LIST) {
                (archive.read_le()?, archive.read_le()?)
            } else {
                (0, 0)
            };

        let localization_id = if archive
            .serialized_with(ObjectVersion::VER_UE4_ADDED_PACKAGE_SUMMARY_LOCALIZATION_ID)
            && editor_only
        {
            Some(UnrealString::parse_inline(&mut archive)?)
        } else {
            None
        };

        let (gatherable_text_data_count, gatherable_text_data_offset) =
            if archive.serialized_with(ObjectVersion::VER_UE4_SERIALIZE_TEXT_IN_PACKAGES) {
                (archive.read_le()?, archive.read_le()?)
            } else {
                (0, 0)
            };

        let exports = UnrealArray::<UnrealObjectExport>::parse_indirect(&mut archive)?;
        let imports = UnrealArray::<UnrealClassImport>::parse_indirect(&mut archive)?;

        // UE5.5 added Verse cell tables — skip them.
        if archive.serialized_with(ObjectVersionUE5::VERSE_CELLS) {
            for _ in 0..4 {
                let _: i32 = archive.read_le()?;
            }
        }

        // UE5.4 added a metadata-section offset.
        if archive.serialized_with(ObjectVersionUE5::METADATA_SERIALIZATION_OFFSET) {
            let _: i32 = archive.read_le()?;
        }

        let depends_offset: i32 = archive.read_le()?;

        let (soft_package_references_count, soft_package_references_offset) =
            if archive.serialized_with(ObjectVersion::VER_UE4_ADD_STRING_ASSET_REFERENCES_MAP) {
                (archive.read_le()?, archive.read_le()?)
            } else {
                (0, 0)
            };

        let searchable_names_offset =
            if archive.serialized_with(ObjectVersion::VER_UE4_ADDED_SEARCHABLE_NAMES) {
                Some(archive.read_le()?)
            } else {
                None
            };

        let thumbnail_table_offset: i32 = archive.read_le()?;

        // UE5.7+ import-type-hierarchy table — skip.
        if archive.serialized_with(ObjectVersionUE5::IMPORT_TYPE_HIERARCHIES) {
            let _count: i32 = archive.read_le()?;
            let _offset: i32 = archive.read_le()?;
        }

        // GUID block. New-format assets folded the package GUID into SavedHash.
        if !new_format {
            UnrealGuid::seek_past(&mut archive)?;
        }
        if archive.serialized_with(ObjectVersion::VER_UE4_ADDED_PACKAGE_OWNER) && editor_only {
            UnrealGuid::seek_past(&mut archive)?;
            if archive.serialized_without(ObjectVersion::VER_UE4_NON_OUTER_PACKAGE_IMPORT) {
                UnrealGuid::seek_past(&mut archive)?;
            }
        }

        // Generations: skipped (we don't expose them).
        let num_generations: i32 = archive.read_le()?;
        let generations_info = ArrayStreamInfo {
            offset: archive.stream_position()?,
            count: num_generations as u64,
        };
        UnrealArray::<UnrealGenerationInfo>::seek_past_with_info(
            &mut archive,
            &generations_info,
        )?;

        let engine_version = if archive
            .serialized_with(ObjectVersion::VER_UE4_ENGINE_VERSION_OBJECT)
        {
            UnrealEngineVersion::parse_inline(&mut archive)?
        } else {
            // Older assets only persisted the changelist; UE rebuilds the rest as 4.0.0.
            let cl: u32 = archive.read_le()?;
            UnrealEngineVersion::from_changelist(cl)
        };

        let compatible_with_engine_version = if archive
            .serialized_with(ObjectVersion::VER_UE4_PACKAGE_SUMMARY_HAS_COMPATIBLE_ENGINE_VERSION)
        {
            UnrealEngineVersion::parse_inline(&mut archive)?
        } else {
            engine_version.clone()
        };

        let compression_flags: u32 = archive.read_le()?;

        let num_compressed_chunks: i32 = archive.read_le()?;
        let chunks_info = ArrayStreamInfo {
            offset: archive.stream_position()?,
            count: num_compressed_chunks as u64,
        };
        UnrealArray::<UnrealCompressedChunk>::seek_past_with_info(&mut archive, &chunks_info)?;

        let package_source: u32 = archive.read_le()?;

        let additional_packages_to_cook =
            UnrealArray::<UnrealString>::parse_inline(&mut archive)?;

        let texture_allocations = if archive.legacy_version > -7 {
            Some(archive.read_le()?)
        } else {
            None
        };

        let asset_registry_data_offset: i32 = archive.read_le()?;
        let bulk_data_start_offset: i64 = archive.read_le()?;

        let world_tile_info_data_offset =
            if archive.serialized_with(ObjectVersion::VER_UE4_WORLD_LEVEL_INFO) {
                let off: i32 = archive.read_le()?;
                if off > 0 {
                    Some(off)
                } else {
                    None
                }
            } else {
                None
            };

        let chunk_ids = {
            let has_chunkid = archive
                .serialized_with(ObjectVersion::VER_UE4_ADDED_CHUNKID_TO_ASSETDATA_AND_UPACKAGE);
            let has_array = has_chunkid
                && archive.serialized_with(
                    ObjectVersion::VER_UE4_CHANGED_CHUNKID_TO_BE_AN_ARRAY_OF_CHUNKIDS,
                );

            if has_array {
                UnrealArray::<i32>::parse_inline(&mut archive)?
            } else if has_chunkid {
                let id: i32 = archive.read_le()?;
                if id >= 0 {
                    vec![id]
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        };

        let (preload_dependency_count, preload_dependency_offset) = if archive
            .serialized_with(ObjectVersion::VER_UE4_PRELOAD_DEPENDENCIES_IN_COOKED_EXPORTS)
        {
            (archive.read_le()?, archive.read_le()?)
        } else {
            (-1, 0)
        };

        let names_referenced_from_export_data_count = if archive
            .serialized_with(ObjectVersionUE5::NAMES_REFERENCED_FROM_EXPORT_DATA)
        {
            archive.read_le()?
        } else {
            names.len() as i32
        };

        let payload_toc_offset = if archive.serialized_with(ObjectVersionUE5::PAYLOAD_TOC) {
            archive.read_le()?
        } else {
            -1i64
        };

        let data_resource_offset = if archive.serialized_with(ObjectVersionUE5::DATA_RESOURCES) {
            let off: i32 = archive.read_le()?;
            if off > 0 {
                Some(off)
            } else {
                None
            }
        } else {
            None
        };

        Ok(Self {
            archive,
            total_header_size,
            package_name,
            package_flags,
            names,
            soft_object_paths_count,
            soft_object_paths_offset,
            localization_id,
            gatherable_text_data_count,
            gatherable_text_data_offset,
            exports,
            imports,
            depends_offset,
            soft_package_references_count,
            soft_package_references_offset,
            searchable_names_offset,
            thumbnail_table_offset,
            engine_version,
            compatible_with_engine_version,
            compression_flags,
            package_source,
            additional_packages_to_cook,
            texture_allocations,
            asset_registry_data_offset,
            bulk_data_start_offset,
            world_tile_info_data_offset,
            chunk_ids,
            preload_dependency_count,
            preload_dependency_offset,
            names_referenced_from_export_data_count,
            payload_toc_offset,
            data_resource_offset,
        })
    }
}

impl<R> AssetHeader<R> {
    /// Case-insensitive scan of the name table. Numeric suffixes (`Foo_3`) are
    /// not currently understood — only the bare name is matched.
    pub fn find_name(&self, query: &str) -> Option<NameReference> {
        let lowered = query.to_lowercase();
        for (i, candidate) in self.names.iter().enumerate() {
            if candidate == query || candidate.to_lowercase() == lowered {
                return Some(NameReference {
                    index: i as u32,
                    number: None,
                });
            }
        }
        None
    }

    /// Look up a `NameReference` and append the `_N` suffix when present.
    pub fn resolve_name(
        &self,
        reference: &NameReference,
    ) -> std::result::Result<Cow<'_, str>, InvalidNameIndexError> {
        let i = reference.index as usize;
        match self.names.get(i) {
            Some(name) => {
                if let Some(num) = reference.number {
                    let mut owned = name.clone();
                    owned.push('_');
                    owned.push_str(&(num.get() - 1).to_string());
                    Ok(Cow::Owned(owned))
                } else {
                    Ok(Cow::Borrowed(name))
                }
            }
            None => Err(InvalidNameIndexError(reference.index)),
        }
    }

    /// Iterator over imported package names — i.e. the asset's package-level
    /// dependencies.
    pub fn package_import_iter(&self) -> ImportIterator<'_, R> {
        ImportIterator::new(self)
    }
}

impl<R> AssetHeader<R>
where
    R: Seek + Read,
{
    /// Stream the asset's thumbnail directory by seeking to its offset and
    /// returning a lazy iterator.
    pub fn thumbnail_iter(&mut self) -> Result<UnrealArrayIterator<'_, UnrealThumbnailInfo, R>> {
        self.archive
            .seek(SeekFrom::Start(self.thumbnail_table_offset as u64))?;
        let info = ArrayStreamInfo::from_current_position(&mut self.archive)?;
        UnrealArrayIterator::new(self, info)
    }
}
