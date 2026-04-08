# Binary Asset Storage in Forge VCS

How Forge stores diff versions of binary assets (`.uasset`, `.umap`) compared to Git and Perforce.

## How Forge Stores Binary Assets

Forge uses a three-layer strategy purpose-built for Unreal Engine files.

### 1. Semantic Content-Defined Chunking

For `.uasset` and `.umap` files, Forge parses the UE asset header and splits the file along logical boundaries:

| Section | Content |
|---------|---------|
| Header chunk | Package summary, name table, import/export tables |
| Per-export chunks | One chunk per export object (meshes, materials, BPs, etc.) |
| Trailing data | Bulk data after all exports |

Non-UE binary files fall back to FastCDC v2020 with 64 KiB min, 256 KiB average, and 1 MiB max chunk sizes. Files under 1 MiB are stored whole without chunking.

Changing one property on one Blueprint node only invalidates that export's chunk. The header, other exports, and trailing data are deduplicated automatically across versions.

### 2. Content-Addressable Storage with BLAKE3 + Zstd

Every chunk is hashed with BLAKE3 and stored at `.forge/objects/{hash[0:2]}/{hash[2:]}`, compressed with zstd (level 3). Deduplication is trivial: if the hash already exists, the write is skipped.

### 3. Structured Diffing and Three-Way Merge

Forge parses tagged properties from UE asset exports and produces property-level diffs:

```
[StaticMeshComponent0] RelativeLocation.X: 100.0 -> 250.0
[PointLight1] Intensity: 5000.0 -> 8000.0
Import added: /Script/Engine.PointLightComponent
```

Three-way merge (`merge_assets(base, ours, theirs)`) detects non-conflicting property changes across branches and reports precise conflicts with export name, property path, and base/ours/theirs values.

## Comparison with Git

| Aspect | Git | Forge |
|--------|-----|-------|
| Storage model | Whole-file snapshots, then packfile delta compression | Content-defined chunks, dedup at chunk level |
| Delta granularity | Binary xdelta between arbitrary object pairs (opaque) | Semantic chunks aligned to UE export boundaries |
| When deltas happen | Only during `git gc` / `git repack` (packfiles) | At `forge add` time -- dedup is immediate |
| Small change to 500 MB .umap | Stores entire new copy until gc; delta is byte-level with no semantic awareness | Only re-stores changed export chunks (KBs to low MBs) |
| Diff output | `Binary files differ` | Property-level: which export, which field, old to new values |
| Merge | Impossible for binaries -- always conflicts | Three-way property-level merge; auto-merge when changes don't overlap |
| Clone size | Packfiles help but deltas are blind to structure | Chunk dedup means shared chunks across all versions are stored once |

Git was designed for text source code. Binary files get no structural understanding. Packfile deltas (xdelta) find byte-level similarities but cannot reason about individual components within a UE asset. A one-property change to a 200 MB level can produce a nearly full-size delta because unrelated bytes shift around the export table.

## Comparison with Perforce

| Aspect | Perforce | Forge |
|--------|----------|-------|
| Storage model | Full file revisions on central server; optional RCS-style diffs for text, full copies for binary | Content-defined chunks with cross-version dedup |
| Binary storage | Full copy per revision; `+C` flag just compresses the whole file | Only changed chunks stored; unchanged exports deduplicated |
| Small change to 500 MB .umap | Stores another ~500 MB (compressed, maybe ~300 MB) | Stores only changed chunks (KBs to low MBs) plus reuses existing chunks |
| Diff output | None for binaries -- P4V shows "files differ" | Property-level diffs showing exactly what changed |
| Merge | Not possible for binary -- exclusive checkout (`+l`) to prevent conflicts | Locks available but also supports concurrent edits with merge |
| Workflow | Exclusive locks to avoid binary conflicts entirely | Locks available but not required when changes don't overlap |
| Scaling | Server stores N full copies for N revisions of a binary | Server stores deduplicated chunk pool; N revisions share most chunks |

Perforce sidesteps the binary problem entirely. Instead of trying to diff or merge binaries, it uses exclusive file locking so only one person can edit a `.uasset` at a time. This works but creates bottlenecks: artists wait for locks, and the server stores a full compressed copy per revision.

## Storage Example: 10 Revisions of a 200 MB .umap

Each revision changes approximately 2% of the file (a few actors moved, one material swapped).

| System | Approximate Server Storage |
|--------|---------------------------|
| Git (loose) | 2 GB (10 full copies) |
| Git (packed) | 400-800 MB (xdelta helps but is structure-blind) |
| Perforce | 1.2-1.5 GB (10 zstd-compressed full copies) |
| Forge | 220-260 MB (base chunks plus 2-4 MB of changed chunks per revision) |

### Why the Difference

The key insight is semantic chunking at UE export boundaries. When an artist moves a StaticMeshActor:

- **Git**: The entire file is a new blob. Byte offsets shift because the export table changes, causing xdelta to struggle.
- **Perforce**: A new full compressed copy is stored.
- **Forge**: Only the modified export's chunk and the header chunk (which contains updated offsets) are new. All other export chunks hash identically and are deduplicated.

## Diffing

Git and Perforce both output `Binary files differ` for UE assets. Forge produces actionable information:

```
[SM_Rock_03] RelativeLocation: (1200, 500, 0) -> (1350, 480, 0)
[MI_Ground] Parent changed: /Game/Mat/Old -> /Game/Mat/New
Property removed: [PointLight_2] AttenuationRadius
```

## Merging

- **Git**: Binary conflict. One artist's work is lost or must be redone manually.
- **Perforce**: Prevents the situation via exclusive locks at the cost of collaboration speed.
- **Forge**: If Artist A moved rocks and Artist B changed lighting, the merge detects these as changes to different exports and properties and can flag them as auto-mergeable.

## Full Storage Pipeline

What happens when running `forge add` on a modified 5 MB `.uasset`:

1. Read file, compute BLAKE3 content hash.
2. Recognize `.uasset` extension, parse asset header and exports.
3. Split into semantic chunks along export boundaries:
   - Header section (512 KB)
   - Export A (1.2 MB)
   - Export B (1.8 MB)
   - Trailing data (1.5 MB)
4. Hash each chunk with BLAKE3, compress with zstd.
5. Write to object store with dedup: if a chunk hash already exists, skip.
6. Write a manifest (ChunkedBlob) listing chunk hashes in order.
7. Update the index with content hash, manifest hash, and chunked flag.

On the next commit, if only Export B changed, sections for the header, Export A, and trailing data hash identically and are not re-stored.

## Current Limitations

1. **Cooked assets** with unversioned properties cannot be parsed. Falls back to FastCDC byte-level chunking, which still provides better dedup than Git or Perforce full copies but loses semantic diffing.
2. **Binary reconstruction after merge** is not yet implemented. Currently reports merge feasibility with full conflict details but cannot produce the merged `.uasset` file.
3. **Cross-file chunk dedup** works naturally through content addressing (same hash = same stored chunk) but has no explicit optimization tracking.
