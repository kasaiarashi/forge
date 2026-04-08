# Binary VCS Strategy for Unreal Engine

A development roadmap for making Forge the best version control system for Unreal Engine binary assets, based on analysis of UE's source code internals and Forge's current implementation.

---

## 1. UE Asset Format Internals

Understanding the binary format is essential for building VCS tooling that can parse, diff, chunk, and merge UE assets intelligently.

### Package Binary Layout

Every `.uasset` and `.umap` file follows this structure:

```
Offset 0x00        FPackageFileSummary
                   ├── Tag: 0x9E2A83C1 (magic number)
                   ├── FileVersionUE (UE4 + UE5 version pair)
                   ├── FileVersionLicenseeUE
                   ├── CustomVersionContainer (array of GUID + version pairs)
                   ├── PackageFlags (cooked, editor-only, unversioned, etc.)
                   ├── TotalHeaderSize
                   ├── NameCount / NameOffset
                   ├── ImportCount / ImportOffset
                   ├── ExportCount / ExportOffset
                   ├── BulkDataStartOffset (int64)
                   ├── PayloadTocOffset (int64, UE5+)
                   ├── DataResourceOffset (int32, UE5+)
                   └── ...additional offsets and metadata

NameOffset          Name Table
                   └── NameCount entries (FName strings with optional hashes)

ImportOffset        Import Table
                   └── ImportCount FObjectImport entries

ExportOffset        Export Table
                   └── ExportCount FObjectExport entries

ExportData          Sequential export object data
                   ├── Export 0: SerialSize bytes at SerialOffset
                   ├── Export 1: SerialSize bytes at SerialOffset
                   └── ...

BulkDataStart       Inline bulk data (textures, meshes, audio)

EOF region          Package Trailer (optional, UE5+)
```

Source: `Engine/Source/Runtime/CoreUObject/Public/UObject/PackageFileSummary.h`

### FObjectExport Key Fields

Each export entry describes one serialized UObject in the package:

| Field | Type | Purpose |
|-------|------|---------|
| ClassIndex | FPackageIndex | Class definition (positive=export, negative=import, 0=null) |
| ObjectName | FName | Object name (index into name table) |
| OuterIndex | FPackageIndex | Parent object in hierarchy |
| SuperIndex | FPackageIndex | Parent class (for UStructs) |
| SerialSize | int64 | Byte count of this export's data |
| SerialOffset | int64 | Absolute file offset to data start |
| ScriptSerializationStartOffset | int64 | Property data start (UE5+, relative to SerialOffset) |
| ScriptSerializationEndOffset | int64 | Property data end (UE5+) |
| ObjectFlags | uint32 | RF_* flags controlling load behavior |

Source: `Engine/Source/Runtime/CoreUObject/Public/UObject/ObjectResource.h`

### Property Serialization

UE has two modes for serializing object properties:

**Tagged properties** (editor/uncooked assets, `bUnversioned=false`):
Each property is prefixed with an FPropertyTag header containing the property name, type, size, and array index. This is version-tolerant -- unknown properties can be skipped by size, and missing properties get default values.

```
FPropertyTag:
  ├── Name (FName)           -- property name
  ├── Type (FName)           -- "IntProperty", "ObjectProperty", "StructProperty", etc.
  ├── Size (int32)           -- byte size of value data
  ├── ArrayIndex (int32)     -- for fixed-size array elements
  ├── PropertyGuid (FGuid)   -- for Blueprint properties
  └── SerializeType (enum)   -- tagged, binary, or skipped
```

**Unversioned properties** (cooked/shipped assets, `bUnversioned=true`):
Properties are serialized as raw binary without tags. Requires the exact class schema (field names, types, order) to interpret. This is faster to load but impossible to parse without the matching UE class definitions.

Source: `Engine/Source/Runtime/CoreUObject/Public/UObject/PropertyTag.h`

### File Splitting

Large assets are split across multiple files:

| Extension | Content | When Used |
|-----------|---------|-----------|
| `.uasset` / `.umap` | Header + tables + small export data | Always present |
| `.uexp` | Export data continuation | When export data exceeds header file |
| `.ubulk` | Large bulk data (textures, meshes) | Cooked packages with separated bulk data |
| `.uptnl` | Optional payload data | Optional quality levels |
| `.m.ubulk` | Memory-mappable bulk data | Platform-specific cooked data |

The `.uexp` file is a direct continuation of the `.uasset` -- export `SerialOffset` values in the header are computed as if `.uexp` data is appended at `TotalHeaderSize`. So the offset within `.uexp` is `SerialOffset - TotalHeaderSize`.

### Bulk Data Storage

Bulk data (textures, meshes, audio) uses the FBulkData system with these storage modes:

| Flag | Location | Description |
|------|----------|-------------|
| `BULKDATA_PayloadAtEndOfFile` | Inline in .uasset | After export data |
| `BULKDATA_PayloadInSeparateFile` | .ubulk file | Separated for streaming |
| `BULKDATA_OptionalPayload` | .uptnl file | Optional quality levels |
| `BULKDATA_MemoryMappedPayload` | .m.ubulk file | Platform-optimized |
| `BULKDATA_SerializeCompressedZLIB` | Any | Zlib compressed |

Source: `Engine/Source/Runtime/CoreUObject/Public/Serialization/BulkData.h`

### UE5 Zen/IoStore Format

Cooked distribution packages use an optimized format:

```
FZenPackageSummary:
  ├── HeaderSize
  ├── Name (FMappedName)
  ├── PackageFlags
  ├── ImportMapOffset / ExportMapOffset
  ├── ExportBundleEntriesOffset    -- command bundling for async loading
  ├── DependencyBundleHeadersOffset
  └── ImportedPackageNamesOffset

FExportMapEntry (fixed-size):
  ├── CookedSerialOffset / CookedSerialSize
  ├── ObjectName (FMappedName)
  ├── OuterIndex / ClassIndex / SuperIndex
  └── PublicExportHash (uint64)
```

Source: `Engine/Source/Runtime/CoreUObject/Internal/Serialization/ZenPackageHeader.h`

### UE's Built-in Diff and Merge Tools

UE's editor includes property-level diff and merge UIs that any VCS can leverage:

- **SBlueprintDiff** -- Visual three-panel Blueprint comparison with graph editor, property details, and linked navigation
- **SMaterialDiff** -- Material graph comparison with viewport preview
- **SBlueprintMerge** -- Three-way merge UI with per-property accept/reject for remote, base, and local versions
- **DiffUtils::CompareUnrelatedObjects()** -- Deep structural comparison engine for any UObject

The VCS does not need to replicate these tools. It only needs to provide file revisions to temp paths, then launch UE's diff/merge UI.

Source: `Engine/Source/Editor/UnrealEd/Public/DiffUtils.h`, `Engine/Source/Editor/Kismet/Public/SBlueprintDiff.h`, `Engine/Source/Developer/Merge/Public/SBlueprintMerge.h`

### UE Source Control Interface

UE expects VCS plugins to implement `ISourceControlProvider` with 20+ operations:

**Core operations:** CheckOut, CheckIn, MarkForAdd, Delete, Revert, Sync, Resolve, Copy, UpdateStatus, Connect

**State queries:** IsCheckedOut, IsModified, IsAdded, IsDeleted, IsConflicted, CanCheckIn, CanCheckout, IsCurrent

**Optional capabilities:** UsesChangelists, UsesCheckout, UsesFileRevisions, UsesSnapshots, AllowsDiffAgainstDepot

**Key insight:** The Perforce plugin uses exclusive file locking, changelist management, and depot path mapping. The Git plugin detects Git LFS and version capabilities. Forge's plugin should expose its unique strengths: semantic diffing, property-level merge, and optional (not mandatory) locking.

Source: `Engine/Source/Developer/SourceControl/Public/ISourceControlProvider.h`

---

## 2. Current Forge Capabilities

### Semantic Chunking (uasset_chunk.rs)

For `.uasset` and `.umap` files, Forge parses the header and splits the file at export boundaries:

- **Header chunk**: Everything up to the first export's data (summary, name table, import/export tables)
- **Per-export chunks**: One chunk per FObjectExport, at its SerialOffset for SerialSize bytes
- **Trailing chunk**: Any data after the last export

Files under 1 MiB are stored whole without chunking. Non-UE files use FastCDC (64 KiB min, 256 KiB avg, 1 MiB max).

### Property Parsing (property.rs)

The tagged property parser supports 32 property types:

**Primitives:** Bool, Int8, Int16, Int, Int64, UInt16, UInt32, UInt64, Float, Double, Str, Text, Name

**References:** Object, Interface, LazyObject, SoftObject, Enum

**Collections:** Array, Map, Set

**Compounds:** Struct (with 16 well-known native structs: Vector, Vector2D, Vector4, Rotator, Quat, LinearColor, Color, IntPoint, Guid, DateTime, Timespan, SoftObjectPath, SoftClassPath, GameplayTag, GameplayTagContainer, Transform)

Unknown types fall back to `Opaque` with raw bytes preserved for round-trip fidelity.

### Property-Level Diffing (uasset_diff.rs)

Produces semantic diffs showing exactly what changed:

```
Import added: /Script/Engine.PointLightComponent
[StaticMeshComponent0] RelativeLocation.X: 100.0 -> 250.0
[PointLight1] Intensity: 5000.0 -> 8000.0
Export removed: OldActor_3
```

Supports recursive struct field comparison and fallback to binary size comparison for unparseable exports.

### Three-Way Merge Detection (uasset_merge.rs)

Detects merge outcomes at the property level:

- **Identical**: Both sides made the same changes
- **TakeOurs/TakeTheirs**: Only one side changed
- **AutoMerged**: Both sides changed, but non-conflicting properties
- **Conflict**: Same property modified differently on both sides
- **CannotMerge**: Parse failure on any version

### Storage Pipeline

```
File on disk
  -> BLAKE3 content hash
  -> Semantic chunking (export boundaries for .uasset/.umap, FastCDC for others)
  -> Zstd compression (level 3) per chunk
  -> Content-addressable storage at .forge/objects/{hash[0:2]}/{hash[2:]}
  -> Dedup: if hash exists, skip write
  -> ChunkedBlob manifest linking chunk hashes in order
  -> Index entry: content_hash, object_hash (manifest), is_chunked flag
```

---

## 3. Critical Gaps

### Gap 1: Cooked Assets Rejected

**Location:** `crates/uasset/src/structured.rs:81`

```rust
if header.package_flags & (PackageFlags::UnversionedProperties as u32) != 0 {
    return Err(StructuredParseError::UnversionedProperties);
}
```

Assets with the `UnversionedProperties` flag (all cooked/shipped assets) are completely rejected. No diffing, no merge detection, no semantic chunking via the structured path. These assets still get FastCDC chunking via the byte-level fallback, but lose all semantic awareness.

**Impact:** Cannot provide meaningful diffs or merges for cooked builds, QA testing workflows, or mod support.

### Gap 2: Split Files Ignored

**Location:** `crates/uasset/src/structured.rs:194`

When an export's `SerialOffset + SerialSize` exceeds the `.uasset` file size (meaning the data is in a `.uexp` file), the export is silently skipped with `properties: None`. This means for any non-trivial asset where UE splits export data into `.uexp`, Forge loses visibility into the actual content.

Additionally, `.ubulk` and `.uptnl` files are not recognized by the chunking system at all -- they fall through to generic FastCDC with no semantic awareness.

**Impact:** For large assets (most levels, complex Blueprints), the majority of data lives in `.uexp`. Forge is effectively blind to it.

### Gap 3: Hardcoded Export Stride Guessing

**Location:** `crates/forge-core/src/uasset_chunk.rs:265`

```rust
let strides = [100, 96, 92, 88, 84, 80, 76, 72, 68, 64, 104, 108];
```

The semantic chunker hand-parses the export table by guessing the per-entry byte stride from a hardcoded list. This is fragile -- new UE versions or custom engine builds may use different export entry sizes. Meanwhile, the `uasset` crate already parses export tables correctly for all supported versions.

**Impact:** Potential silent failures on future UE versions where export entry size changes.

### Gap 4: No Binary Reconstruction After Merge

**Location:** `crates/forge-cli/src/commands/merge.rs:148-150`

```rust
// For now, since we can't reconstruct the binary,
// flag as conflict with details.
asset_conflicts.push(((*path).clone(), ours_desc, theirs_desc));
```

When Forge detects that two branches made non-conflicting property changes (AutoMerged), it still reports the file as conflicted because it cannot produce the merged binary. The merge detection works, but the output is useless without reconstruction.

**Impact:** The biggest missed opportunity. Artists making independent changes to the same asset still get conflicts, even when Forge knows the merge is safe.

### Gap 5: No Bulk Data Awareness

Texture mip chains, mesh LODs, and audio data in `.ubulk` files get generic FastCDC chunking with no understanding of internal structure. Texture mip levels are natural chunk boundaries -- lower mips rarely change when an artist edits the highest resolution.

**Impact:** Suboptimal deduplication for the largest files in a typical UE project (textures are often 50%+ of total asset size).

---

## 4. Comparison with Git and Perforce

### Storage Efficiency

| Scenario: 10 revisions of 200 MB .umap (2% change each) | Storage |
|---|---|
| Git (loose objects) | ~2 GB (10 full copies) |
| Git (packed, after gc) | ~400-800 MB (xdelta, structure-blind) |
| Perforce | ~1.2-1.5 GB (10 zstd-compressed full copies) |
| Forge (current) | ~220-260 MB (semantic chunks, dedup across versions) |
| Forge (with split-file support) | ~200-240 MB (includes .uexp dedup) |

### Diffing

| System | Binary Diff Output |
|--------|-------------------|
| Git | `Binary files differ` |
| Perforce | `files differ` (P4V shows nothing for binaries) |
| Forge (current) | Property-level: `[Actor] Location.X: 100 -> 250` (for assets with inline export data) |
| Forge (with .uexp support) | Same property-level output for all uncooked assets regardless of file splitting |

### Merging

| System | Binary Merge |
|--------|-------------|
| Git | Always conflicts. One side's work is lost. |
| Perforce | Prevents via exclusive locks. No concurrent editing. |
| Forge (current) | Detects safe merges but cannot produce the result -- still conflicts. |
| Forge (with reconstruction) | Auto-merges non-conflicting property changes, produces valid binary output. |

### Where UE's Own Tools Help

UE's editor has visual diff/merge that no external VCS can replicate (Blueprint graph diff, material node diff, three-way merge UI). The VCS doesn't need to -- it just needs to:

1. Extract file revisions to temp paths
2. Launch UE's diff: `UE4Editor.exe -diff <left> <right>`
3. Launch UE's merge: provide base/local/remote paths to `SBlueprintMerge`

Forge's CLI-level property diffs complement this by working without UE installed (CI, code review, terminal workflows).

---

## 5. Strategy: Phased Implementation

### Phase 1: Asset Group Awareness

**Goal:** Treat `.uasset` + `.uexp` + `.ubulk` + `.uptnl` as a single logical asset.

**Why first:** This is the foundation for everything else. Without it, export data in `.uexp` files (where 90%+ of actual content lives for large assets) is invisible to diffing, merging, and semantic chunking.

**Changes:**

1. **New file `crates/forge-core/src/asset_group.rs`:**
   - `AssetGroup` struct: `header_path`, `uexp_path: Option`, `ubulk_path: Option`, `uptnl_path: Option`
   - `resolve_asset_group(path) -> AssetGroup`: given any companion file, find the others by extension substitution
   - `load_combined_data(group, root) -> CombinedAssetData`: read header + uexp as a combined byte stream for parsing

2. **Modify `crates/uasset/src/structured.rs`:**
   - Change `parse_structured(data: &[u8])` to `parse_structured(header_data: &[u8], uexp_data: Option<&[u8]>)`
   - When `serial_offset + serial_size > header_data.len()`, read from `uexp_data` at offset `serial_offset - total_header_size`
   - No longer silently skip split exports

3. **Modify `crates/forge-cli/src/commands/diff.rs`:**
   - When diffing a `.uasset`, also load companion `.uexp` from object store
   - Suppress standalone `.uexp`/`.ubulk` diff entries; annotate the parent `.uasset` instead

4. **Modify `crates/forge-cli/src/commands/merge.rs`:**
   - Load companion files for all three versions (base, ours, theirs) before merge

5. **Modify `crates/forge-core/src/chunk.rs`:**
   - Recognize `.uexp`/`.ubulk`/`.uptnl` extensions
   - For `.uexp`: use FastCDC initially (Phase 4c upgrades to export-boundary splitting)

**Files:** `asset_group.rs` (new), `structured.rs`, `diff.rs`, `merge.rs`, `chunk.rs`, `forge-core/src/lib.rs`

### Phase 2: Robust Export Table Parsing

**Goal:** Replace hardcoded stride guessing with proper parsing from the `uasset` crate.

**Why second:** Quick win that eliminates the most fragile code in the system. The `uasset` crate already parses export tables correctly -- the semantic chunker just needs to use it.

**Changes:**

1. **Refactor `crates/forge-core/src/uasset_chunk.rs`:**
   - Replace `extract_sections()` and the entire `read_export_regions()` / `try_read_exports_at_offsets()` / `try_with_stride()` chain with:
   ```rust
   fn extract_sections(data: &[u8]) -> Option<Vec<Section>> {
       let header = AssetHeader::new(Cursor::new(data)).ok()?;
       let header_end = header.total_header_size as usize;
       // Use header.exports directly for SerialOffset/SerialSize
       // No stride guessing needed
   }
   ```
   - Delete the `strides` array and all stride-guessing logic (~100 lines removed)

2. **Add `uasset` dependency to `crates/forge-core/Cargo.toml`** if not already present.

**Files:** `uasset_chunk.rs` (major simplification), `forge-core/Cargo.toml`

### Phase 3: Binary Merge Reconstruction

**Goal:** When merge detects non-conflicting changes, produce the actual merged binary file.

**Why third:** This is the single most impactful user-facing feature. It turns "both sides changed this file, you lose" into "auto-merged, here's the result."

**Changes:**

1. **Add property serializer to `crates/uasset/src/property.rs`:**
   - `serialize_tagged_properties(props: &[TaggedProperty], names: &[String]) -> Vec<u8>`
   - Inverse of `parse_tagged_properties`: write FPropertyTag header + value bytes for each property
   - Well-known structs (Vector, Rotator, etc.) serialize in native binary format
   - `Opaque` variant writes raw bytes verbatim (unknown types round-trip correctly)

2. **New file `crates/forge-core/src/uasset_reconstruct.rs`:**
   - `reconstruct_merged(base: &[u8], modifications: &[ExportModification]) -> Result<Vec<u8>>`
   - Algorithm:
     1. Parse base header to get name table, export table, section boundaries
     2. For each modified export, serialize merged property list
     3. Compute delta in serial sizes
     4. Rewrite file: copy header, update export table SerialOffset/SerialSize fields (offsets shift when preceding exports change size), update BulkDataStartOffset and PayloadTocOffset
     5. For unmodified exports: copy bytes verbatim from base
     6. For modified exports: write new property bytes + copy trailing native data from base

3. **Modify `crates/forge-core/src/uasset_merge.rs`:**
   - `AutoMerged` variant returns `merged_exports: Vec<MergedExport>` with the actual merged property data

4. **Modify `crates/forge-cli/src/commands/merge.rs`:**
   - On `AutoMerged`, call reconstructor using "ours" as the base binary, apply "theirs" non-conflicting changes
   - Store result as a new blob instead of reporting conflict

**Files:** `property.rs`, `uasset_reconstruct.rs` (new), `uasset_merge.rs`, `merge.rs`, `forge-core/src/lib.rs`

### Phase 4: Cooked Asset Support

**Goal:** Handle assets with `UnversionedProperties` flag (cooked/shipped builds).

**Why fourth:** Important for QA and mod workflows, but most editor-based development uses uncooked assets. Phases 1-3 have higher impact.

**Changes:**

1. **Header-only mode in `crates/uasset/src/structured.rs`:**
   - Instead of returning `Err(UnversionedProperties)`, return a `StructuredAsset` with `properties: None` for all exports
   - Enables import/export-level diffing (added/removed objects), binary size tracking, and semantic chunking

2. **Export-boundary chunking for `.uexp`:**
   - In `chunk.rs`, when chunking a `.uexp`, accept optional companion header bytes
   - Parse header for export offsets, compute `.uexp`-relative positions as `serial_offset - total_header_size`
   - Split `.uexp` at export boundaries for per-export chunk stability

3. **Optional future: schema-based unversioned parsing:**
   - Ship `.usmap` schema files extracted from UE (via UnrealHeaderTool or FModel)
   - Parse schemas to reconstruct field order for unversioned deserialization
   - Deferred -- high effort, narrow benefit compared to header-only mode

**Files:** `structured.rs`, `chunk.rs`, `uasset_chunk.rs`

### Phase 5: Bulk Data Strategy

**Goal:** Type-specific chunking for `.ubulk` files to maximize deduplication.

**Why fifth:** Optimization phase. Generic FastCDC already works for `.ubulk`; this makes it better for the largest files.

**Changes:**

1. **Parse bulk data TOC from header in `crates/uasset/src/lib.rs`:**
   - Read `PayloadTocOffset` entries (UE5+) or inline bulk data chunk headers
   - Expose `BulkDataEntry { flags, offset, size, raw_size }` for each bulk data block

2. **New file `crates/forge-core/src/bulk_chunk.rs`:**
   - Texture mip-level splitting: each mip is a natural chunk boundary. Lower mips rarely change when artists edit the highest resolution -- gives excellent cross-version dedup.
   - Mesh buffer splitting: vertex buffer, index buffer, LOD boundaries
   - Audio: default FastCDC (no special structure to exploit)

3. **Dispatch in `crates/forge-core/src/chunk.rs`:**
   - `.ubulk` files use `bulk_chunk` when companion header is available
   - Falls back to FastCDC when header is unavailable or unparseable

**Files:** `lib.rs` (uasset), `bulk_chunk.rs` (new), `chunk.rs`

### Phase 6: UE Editor Integration

**Goal:** Let UE's built-in diff/merge tools work with Forge revisions.

**Why last:** CLI workflows already work. This completes the UE editor experience.

**Changes:**

1. **Add `--extract` mode to `crates/forge-cli/src/commands/diff.rs`:**
   - Given two revisions and a path, write both versions to temp files, print their paths
   - UE's diff can then be launched: `UE4Editor.exe -diff <left> <right>`

2. **Enhance the UE source control plugin (`plugin/ForgeSourceControl/`):**
   - Map `ISourceControlProvider` operations to Forge CLI calls:
     - `GetState` -> `forge status --json`
     - `CheckOut` -> `forge lock`
     - `CheckIn` -> `forge snapshot && forge push`
     - `Revert` -> `forge restore`
     - `Diff` -> `forge diff --extract` then launch UE's diff UI
     - `Resolve` -> `forge merge --resolve`
   - Expose Forge capabilities: `UsesCheckout=true` (optional locking), `UsesFileRevisions=true`, `AllowsDiffAgainstDepot=true`

**Files:** `diff.rs`, UE plugin C++ code (outside Rust codebase)

### Phase Dependency Chain

```
Phase 1 (Asset Groups)        -- Foundation: unlocks split-file support
    |
    v
Phase 2 (Robust Parsing)      -- Quick cleanup: eliminates fragile heuristics
    |
    v
Phase 3 (Merge Reconstruction) -- Biggest user impact: real auto-merge
    |
    v
Phase 4 (Cooked Assets)       -- Widens coverage to shipped builds
    |
    v
Phase 5 (Bulk Data)           -- Storage optimization for large assets
    |
    v
Phase 6 (UE Integration)      -- Editor workflow completion
```

---

## 6. Key Architectural Decisions

### Asset grouping belongs in forge-core, not uasset

The `uasset` crate is a focused parser library. Asset grouping (`.uasset` + `.uexp` + `.ubulk` as one logical unit) is a VCS concept. Keep it in `forge-core/src/asset_group.rs`. The `uasset` crate just gets a second parameter for `.uexp` data.

### Reconstruct from "ours" base, not from scratch

When producing a merged binary, start with the "ours" version as a known-valid base. Only modify the specific exports with merged properties, copying everything else byte-for-byte. This avoids needing to reserialize the entire file (which would require handling every UE version's header quirks perfectly). It also means the output is guaranteed to have valid header structure -- only export data regions change.

### Keep .uexp and .uasset as separate objects in the store

They're already separate files on disk and in the tree. Group awareness is a presentation/logic layer, not a storage layer. This avoids breaking existing repos and keeps the object store simple.

### Header-only mode for cooked assets, defer schema parsing

Full unversioned property parsing would require maintaining a database of UE class schemas that changes every engine version. Header-only mode (exports without property details) still enables export-level diffing, semantic chunking, and bulk data awareness -- the most impactful features. Schema parsing can be added later if demand justifies the maintenance cost.

### Leverage UE's visual diff tools, don't replicate them

UE's Blueprint diff viewer, material diff viewer, and three-way merge UI are sophisticated tools that would take months to replicate. Forge should provide the file extraction and revision retrieval that these tools need, then launch them. Forge's own CLI property diffs serve a complementary role for terminal workflows, CI pipelines, and code review without UE installed.
