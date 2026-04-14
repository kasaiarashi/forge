//! Semantic chunking for UE assets.
//!
//! Instead of blind FastCDC on raw bytes, splits `.uasset` files by logical
//! sections (header, name table, per-export data). This means changing one
//! property on one object only invalidates that export's chunk, not the whole file.

use crate::hash::ForgeHash;
use crate::object::blob::{ChunkRef, ChunkedBlob};
use std::io::Cursor;

/// Result of attempting semantic chunking on a UE asset.
pub enum SemanticChunkResult {
    /// Successfully split into semantic sections.
    Chunked {
        manifest: ChunkedBlob,
        chunks: Vec<(ForgeHash, Vec<u8>)>,
    },
    /// Could not parse as a UE asset — caller should fall back to FastCDC.
    NotApplicable,
}

/// Attempt to chunk a `.uasset` file by its logical sections.
///
/// Returns `SemanticChunkResult::NotApplicable` if the data can't be parsed
/// as a valid UE asset header, in which case the caller should fall back to
/// content-defined chunking.
pub fn chunk_uasset(data: &[u8]) -> SemanticChunkResult {
    let sections = match extract_sections(data) {
        Some(s) => s,
        None => return SemanticChunkResult::NotApplicable,
    };

    let mut chunks = Vec::new();
    let mut refs = Vec::new();
    let mut offset = 0u64;

    for section in &sections {
        let section_data = &data[section.start..section.end];
        if section_data.is_empty() {
            continue;
        }
        let hash = ForgeHash::from_bytes(section_data);
        let size = section_data.len() as u64;

        refs.push(ChunkRef { hash, size, offset });
        chunks.push((hash, section_data.to_vec()));
        offset += size;
    }

    // Verify total coverage equals file size.
    if offset as usize != data.len() {
        return SemanticChunkResult::NotApplicable;
    }

    SemanticChunkResult::Chunked {
        manifest: ChunkedBlob {
            total_size: data.len() as u64,
            chunks: refs,
        },
        chunks,
    }
}

/// Attempt to chunk a `.uexp` file using export boundaries from its companion header.
///
/// The `.uexp` file is a continuation of the `.uasset` header. Export `serial_offset`
/// values are absolute (relative to the combined .uasset+.uexp stream), so we compute
/// `.uexp`-relative offsets as `serial_offset - total_header_size`.
pub fn chunk_uexp_with_header(uexp_data: &[u8], header_data: &[u8]) -> SemanticChunkResult {
    let sections = match extract_uexp_sections(uexp_data, header_data) {
        Some(s) => s,
        None => return SemanticChunkResult::NotApplicable,
    };

    let mut chunks = Vec::new();
    let mut refs = Vec::new();
    let mut offset = 0u64;

    for section in &sections {
        let section_data = &uexp_data[section.start..section.end];
        if section_data.is_empty() {
            continue;
        }
        let hash = ForgeHash::from_bytes(section_data);
        let size = section_data.len() as u64;

        refs.push(ChunkRef { hash, size, offset });
        chunks.push((hash, section_data.to_vec()));
        offset += size;
    }

    if offset as usize != uexp_data.len() {
        return SemanticChunkResult::NotApplicable;
    }

    SemanticChunkResult::Chunked {
        manifest: ChunkedBlob {
            total_size: uexp_data.len() as u64,
            chunks: refs,
        },
        chunks,
    }
}

struct Section {
    start: usize,
    end: usize,
}

/// Parse the UE asset header using the `uasset` crate to extract section boundaries.
///
/// Uses proper header parsing instead of hardcoded stride guessing. The `uasset` crate
/// handles all UE4/UE5 version differences correctly.
fn extract_sections(data: &[u8]) -> Option<Vec<Section>> {
    if data.len() < 40 {
        return None;
    }

    // Validate magic before full parse.
    let magic = u32::from_le_bytes(data[0..4].try_into().ok()?);
    if magic != 0x9E2A83C1 {
        return None;
    }

    let cursor = Cursor::new(data);
    let header = forge_unreal::AssetHeader::new(cursor).ok()?;

    let mut sections = Vec::new();

    // Section 1: Everything up to the first export's data (the "metadata header").
    let header_end = (header.total_header_size as usize).min(data.len());

    // Read export serial offsets/sizes directly from the parsed header.
    let mut export_regions: Vec<(usize, usize)> = header
        .exports
        .iter()
        .map(|e| (e.serial_offset as usize, e.serial_size as usize))
        .filter(|(_off, size)| *size > 0)
        .collect();

    if export_regions.is_empty() {
        // No exports — just split header vs rest.
        sections.push(Section { start: 0, end: header_end });
        if header_end < data.len() {
            sections.push(Section { start: header_end, end: data.len() });
        }
        return Some(sections);
    }

    // Sort export regions by offset.
    export_regions.sort_by_key(|&(off, _)| off);

    // Build sections:
    // 1. Header (0 to first export data or total_header_size)
    let first_export_start = export_regions
        .iter()
        .map(|(off, _)| *off)
        .filter(|&off| off > 0 && off < data.len())
        .min()
        .unwrap_or(header_end);

    let metadata_end = first_export_start.min(data.len());
    sections.push(Section { start: 0, end: metadata_end });

    // 2. Gap between metadata end and first export (if any).
    if metadata_end < first_export_start && first_export_start < data.len() {
        sections.push(Section { start: metadata_end, end: first_export_start });
    }

    // 3. Per-export data sections.
    let mut last_end = first_export_start;
    for &(off, size) in &export_regions {
        if off >= data.len() {
            break; // Export data is in .uexp — stop.
        }
        let end = (off + size).min(data.len());
        if off > last_end {
            // Gap between exports.
            sections.push(Section { start: last_end, end: off });
        }
        if end > off {
            sections.push(Section { start: off, end });
        }
        last_end = end;
    }

    // 4. Trailing data after all exports.
    if last_end < data.len() {
        sections.push(Section { start: last_end, end: data.len() });
    }

    Some(sections)
}

/// Extract section boundaries for a `.uexp` file using its companion `.uasset` header.
fn extract_uexp_sections(uexp_data: &[u8], header_data: &[u8]) -> Option<Vec<Section>> {
    if uexp_data.is_empty() || header_data.len() < 40 {
        return None;
    }

    let cursor = Cursor::new(header_data);
    let header = forge_unreal::AssetHeader::new(cursor).ok()?;
    let total_header_size = header.total_header_size as usize;

    // Find exports whose data lives in the .uexp file (offset >= total_header_size).
    let mut uexp_regions: Vec<(usize, usize)> = header
        .exports
        .iter()
        .filter_map(|e| {
            let abs_offset = e.serial_offset as usize;
            let size = e.serial_size as usize;
            if abs_offset >= total_header_size && size > 0 {
                // Convert to .uexp-relative offset.
                let rel_offset = abs_offset - total_header_size;
                Some((rel_offset, size))
            } else {
                None
            }
        })
        .collect();

    if uexp_regions.is_empty() {
        // No exports in .uexp — just one big section.
        return Some(vec![Section { start: 0, end: uexp_data.len() }]);
    }

    uexp_regions.sort_by_key(|&(off, _)| off);

    let mut sections = Vec::new();
    let mut last_end = 0usize;

    for &(off, size) in &uexp_regions {
        if off >= uexp_data.len() {
            break;
        }
        let end = (off + size).min(uexp_data.len());
        if off > last_end {
            // Gap before this export.
            sections.push(Section { start: last_end, end: off });
        }
        if end > off {
            sections.push(Section { start: off, end });
        }
        last_end = end;
    }

    // Trailing data.
    if last_end < uexp_data.len() {
        sections.push(Section { start: last_end, end: uexp_data.len() });
    }

    Some(sections)
}
