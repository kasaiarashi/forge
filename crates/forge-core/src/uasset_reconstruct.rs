//! Binary reconstruction for merged UE assets.
//!
//! After a three-way property-level merge identifies non-conflicting changes,
//! this module produces the actual merged `.uasset` binary by patching the
//! base file's export data regions with new serialized property data.

use std::collections::HashMap;
use std::io::Cursor;

/// A modification to apply to a specific export during reconstruction.
pub struct ExportModification {
    /// Export index (position in the export table).
    pub export_index: usize,
    /// New serialized property data (output of `serialize_tagged_properties`).
    pub new_property_data: Vec<u8>,
}

/// Reconstruct a `.uasset` file by applying property modifications to exports.
///
/// Takes the base version (known-valid binary) and applies non-conflicting
/// property changes by replacing export data regions. Each modification
/// provides new serialized property bytes for a single export; unchanged
/// exports are copied verbatim from the base.
///
/// The algorithm handles the absolute-offset nature of the format:
/// when an export's data changes size, all subsequent exports' `SerialOffset`
/// values shift by the cumulative delta, as do `BulkDataStartOffset` and
/// `PayloadTocOffset`.
///
/// Returns the reconstructed file bytes, or `None` if reconstruction fails
/// (e.g. the base cannot be parsed).
pub fn reconstruct_merged(
    base_data: &[u8],
    modifications: &[ExportModification],
) -> Option<Vec<u8>> {
    let cursor = Cursor::new(base_data);
    let header = forge_unreal::AssetHeader::new(cursor).ok()?;

    if header.exports.is_empty() {
        return Some(base_data.to_vec());
    }

    let total_header_size = header.total_header_size as usize;

    // Build modification lookup.
    let mod_map: HashMap<usize, &ExportModification> =
        modifications.iter().map(|m| (m.export_index, m)).collect();

    // Sort exports by serial_offset to process in file order.
    let mut export_order: Vec<usize> = (0..header.exports.len()).collect();
    export_order.sort_by_key(|&i| header.exports[i].serial_offset);

    // Phase 1: Build new export data and compute size deltas.
    struct ExportPlan {
        old_offset: usize,
        old_size: usize,
        new_data: Vec<u8>,
        new_size: usize,
    }

    let mut plans: Vec<ExportPlan> = Vec::new();

    for &idx in &export_order {
        let exp = &header.exports[idx];
        let old_offset = exp.serial_offset as usize;
        let old_size = exp.serial_size as usize;

        if old_offset >= base_data.len() {
            continue; // Export lives in .uexp, skip it.
        }

        if let Some(modification) = mod_map.get(&idx) {
            // Modified export: pre-property native data + new properties + trailing native data.
            let prop_start_rel = exp.script_serialization_start_offset as usize;
            let prop_end_rel = exp.script_serialization_end_offset as usize;

            // Pre-property native data (bytes before the tagged property region).
            let pre_prop = if prop_start_rel > 0 {
                let pre_end = old_offset + prop_start_rel;
                if pre_end <= base_data.len() {
                    base_data[old_offset..pre_end].to_vec()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            // Trailing native data (bytes after the tagged property region).
            let trailing_data = if prop_end_rel > 0 && prop_end_rel < old_size {
                let trailing_start = old_offset + prop_end_rel;
                let trailing_end = old_offset + old_size;
                if trailing_start <= base_data.len() && trailing_end <= base_data.len() {
                    base_data[trailing_start..trailing_end].to_vec()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            let mut new_data = Vec::with_capacity(
                pre_prop.len() + modification.new_property_data.len() + trailing_data.len(),
            );
            new_data.extend_from_slice(&pre_prop);
            new_data.extend_from_slice(&modification.new_property_data);
            new_data.extend_from_slice(&trailing_data);

            let new_size = new_data.len();
            plans.push(ExportPlan {
                old_offset,
                old_size,
                new_data,
                new_size,
            });
        } else {
            // Unmodified: copy verbatim from base.
            let end = (old_offset + old_size).min(base_data.len());
            let data = base_data[old_offset..end].to_vec();
            plans.push(ExportPlan {
                old_offset,
                old_size,
                new_data: data,
                new_size: old_size,
            });
        }
    }

    // Phase 2: Build the output byte stream.
    let mut output = Vec::with_capacity(base_data.len());

    // Copy header verbatim.
    let header_end = total_header_size.min(base_data.len());
    output.extend_from_slice(&base_data[..header_end]);

    // Gap between header end and first export (alignment padding, etc.).
    if let Some(first_plan) = plans.first() {
        if first_plan.old_offset > header_end {
            output.extend_from_slice(&base_data[header_end..first_plan.old_offset]);
        }
    }

    // Write export data, preserving any gaps between exports.
    let mut last_old_end = plans
        .first()
        .map(|p| p.old_offset)
        .unwrap_or(header_end);

    for (i, plan) in plans.iter().enumerate() {
        // Preserve gap between previous export end and this export start.
        if plan.old_offset > last_old_end && i > 0 {
            output.extend_from_slice(&base_data[last_old_end..plan.old_offset]);
        }

        output.extend_from_slice(&plan.new_data);
        last_old_end = plan.old_offset + plan.old_size;
    }

    // Trailing data after all exports (bulk data, etc.).
    if last_old_end < base_data.len() {
        output.extend_from_slice(&base_data[last_old_end..]);
    }

    // Phase 3: Patch export table offsets in the header region.
    //
    // Strategy: for each export whose SerialOffset or SerialSize changed,
    // find the old i64 value within the header region and replace it with
    // the new value. We restrict the search to the header region to avoid
    // false matches in export data.
    let mut cumulative_delta: i64 = 0;
    let mut patches: Vec<(i64, i64, i64, i64)> = Vec::new(); // (old_offset, new_offset, old_size, new_size)

    for (plan_idx, &exp_idx) in export_order.iter().enumerate() {
        let exp = &header.exports[exp_idx];
        if (exp.serial_offset as usize) >= base_data.len() {
            continue;
        }

        let new_offset = exp.serial_offset + cumulative_delta;
        let new_size = plans[plan_idx].new_size as i64;

        patches.push((exp.serial_offset, new_offset, exp.serial_size, new_size));
        cumulative_delta += new_size - exp.serial_size;
    }

    // Apply serial offset/size patches within the header region.
    for &(old_off, new_off, old_sz, new_sz) in &patches {
        if old_off != new_off {
            patch_i64_in_region(&mut output, 0, header_end, old_off, new_off);
        }
        if old_sz != new_sz {
            patch_i64_in_region(&mut output, 0, header_end, old_sz, new_sz);
        }
    }

    // Patch BulkDataStartOffset.
    if cumulative_delta != 0 && header.bulk_data_start_offset > 0 {
        let new_bulk = header.bulk_data_start_offset + cumulative_delta;
        patch_i64_in_region(
            &mut output,
            0,
            header_end,
            header.bulk_data_start_offset,
            new_bulk,
        );
    }

    // Patch PayloadTocOffset.
    if cumulative_delta != 0 && header.payload_toc_offset > 0 {
        let new_payload = header.payload_toc_offset + cumulative_delta;
        patch_i64_in_region(
            &mut output,
            0,
            header_end,
            header.payload_toc_offset,
            new_payload,
        );
    }

    Some(output)
}

/// Find the first occurrence of `old_val` (as little-endian i64) within
/// `data[start..end]` and replace it with `new_val`.
fn patch_i64_in_region(data: &mut [u8], start: usize, end: usize, old_val: i64, new_val: i64) {
    let old_bytes = old_val.to_le_bytes();
    let new_bytes = new_val.to_le_bytes();
    let end = end.min(data.len());

    if end < start + 8 {
        return;
    }

    for i in start..=(end - 8) {
        if data[i..i + 8] == old_bytes {
            data[i..i + 8].copy_from_slice(&new_bytes);
            return; // Only replace the first occurrence.
        }
    }
}
