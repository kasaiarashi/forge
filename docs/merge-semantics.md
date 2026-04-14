# Merge Semantics for `.uasset` Diffs

Status: **design only**. No merge implementation exists in Forge yet. This doc
records the thinking behind how structured asset diffs (see
`crates/forge-core/src/uasset_diff.rs` and `crates/forge-core/src/k2node.rs`)
should drive a three-way merge in the future.

## 1. Scope: what kinds of changes are on the table

Post the K2Node pin decoder, the structured diff surfaces these categories:

| Category              | Producer                           | Example                                   |
| --------------------- | ---------------------------------- | ----------------------------------------- |
| Import add/remove     | package header diff                | `+ import: MyStruct`                      |
| Export add/remove     | package header diff                | `+ K2Node_CallFunction_42 (K2Node_CallFunction)` |
| Tagged-property value | `diff_properties`                  | `NodePosX: 100 -> 250`                    |
| Field definition      | `diff_field_definitions`           | `+ variable: Health (float)`              |
| Enum value add/remove | `diff_enum_values`                 | `+ enum: Dead`                            |
| Pin add/remove        | `k2node::parse_k2_node` + matcher  | `+ pin "NewInput" (float, no default)`    |
| Pin rename            | pin matched by `PinId`             | `~ pin renamed: "In" -> "Value"`          |
| Pin type change       | pin matched by `PinId`             | `~ pin "X" type: float -> int`            |
| Pin default change    | pin matched by `PinId`             | `~ pin "X" default: 0 -> 1`               |
| Pin connection change | pin matched by `PinId`             | `~ pin "Exec" connections: 1 -> 2`        |

Everything else (opaque export changes, unparseable nodes) remains coarse-grained.

## 2. Three-way merge model

Define the merge relation as follows. For a file with structured diffs
`D(base, ours) = Δo` and `D(base, theirs) = Δt`:

- **Non-overlapping**: `Δo` and `Δt` touch disjoint entities. Auto-merge =
  apply both change sets in any order.
- **Identical**: `Δo == Δt` on the same entity. Auto-merge = take either (they
  agree).
- **Conflicting**: both touch the same entity with different values. Human
  resolution required.

Entity granularity determines how often we hit conflicts. The right entity
scope here is:

- **Export-level** — `(package, export.object_name)` — for top-level
  add/remove decisions.
- **Property path** — `(export, property_path)` — for tagged-property values.
  Paths like `VarType.PinCategory` already exist in the diff output and are
  the natural merge keys.
- **Pin identity** — `(export, pin_id)` — `FGuid`-stable across renames/type
  changes/reorders. This is what makes pin-level merge feasible: we are not
  diffing by array index.
- **Enum value name** — `(enum_export, value_name)` — the name is the key.
- **Variable name** — `(blueprint_export, field_name)`.

## 3. Auto-merge safety table

Safe means: we can write a merged asset without asking the user.

| Change on ours              | Change on theirs           | Safe? | Notes |
| --------------------------- | -------------------------- | :---: | ----- |
| Import added (different)    | Import added (different)   |  ✓   | Independent. |
| Import added (same)         | —                           |  ✓   | Take either. |
| Export added (different)    | Export added (different)   |  ✓   | Disjoint. |
| Export added (same name, same class) | same       |  △   | Only safe if *contents* match; otherwise structural conflict. |
| Export removed              | Export modified (any kind) |  ✗   | **Delete-vs-edit conflict** — user must decide. |
| Property A changed          | Property B changed         |  ✓   | Different paths, same export. |
| Property A changed          | Property A changed (same)  |  ✓   | Equal values. |
| Property A: v1→v2           | Property A: v1→v3          |  ✗   | Value conflict. |
| Pin added (new PinId)       | Pin added (different PinId)|  ✓   | Different identities. |
| Pin P renamed               | Pin P default changed      |  ✓   | Orthogonal fields. |
| Pin P renamed ours          | Pin P renamed theirs       |  ✗   | Name conflict unless equal. |
| Pin P connections 1→2       | Pin P connections 1→3      |  ✗   | Wiring conflict. |
| Pin P connections 1→2 (add A)| Pin P connections 1→2 (add B) | △ | **Set-union may be safe for LinkedTo** — see §5. |
| Pin P removed               | Pin P default changed      |  ✗   | Delete-vs-edit. |
| Field (variable) added (different names) | same          |  ✓   | Disjoint. |
| Field added, same name, different type   | —              |  ✗   | Type conflict. |
| Enum value added (different)| Enum value added           |  ✓   | Name-keyed set. |
| Enum value removed ours     | Enum value A referenced in theirs | △ | Cross-asset: requires repo-wide reference check. |

## 4. Minimal conflict set requiring human resolution

A merge must pause for the user on:

1. **Delete-vs-edit** on any export, pin, variable, or enum value.
2. **Concurrent value divergence** on the same property path or pin field
   (`name`, `category`, `default`).
3. **Pin connection conflicts** where both sides changed `LinkedTo` in
   incompatible ways — concretely, a pin ID appears in ours but not theirs or
   vice versa *and* the other side's change wasn't a pure superset.
4. **Class changes on an export** — if the export's `class_name` itself
   changed on one side. This is rare but destructive.
5. **Unparseable trailing data** on one side only — we have no way to verify
   non-interference.

Anything outside this list should be auto-mergeable.

## 5. Pin connections: set-union vs conflict

`LinkedTo` is semantically a *set* of references, not an ordered list.
Suppose a pin `P` on node `N` has base `LinkedTo = {A, B}`. Ours adds `C` →
`{A, B, C}`. Theirs adds `D` → `{A, B, D}`.

- **Naive**: diff sees count 2→3 on both sides, flags as conflict.
- **Correct**: compute set-diff against base. `ours_added = {C}`,
  `theirs_added = {D}`, both `removed` empty → merge = `{A, B, C, D}`. Safe.
- **Real conflict**: if ours replaces `B` with `C` (`removed={B}, added={C}`)
  and theirs replaces `B` with `D`, we have a remove-vs-remove that agrees
  plus an add-vs-add that disagrees — flag.

This requires the merge engine to diff LinkedTo element-wise (by the
`PinRef { owning_node, pin_id }` pair) against base rather than by count.
The current diff surfaces only counts; **for merge we will need to expose
the full LinkedTo sets** in `AssetChange` or a dedicated merge-oriented
representation.

## 6. Representation in Forge's data model

Recommended shape for a merge descriptor:

```rust
pub struct AssetMerge {
    pub package: PathBuf,
    pub base_hash: ForgeHash,
    pub ours_hash: ForgeHash,
    pub theirs_hash: ForgeHash,
    pub per_export: HashMap<ExportKey, ExportMerge>,
}

pub struct ExportKey {
    pub object_name: String,
    // class_name intentionally not part of key — class change = conflict itself.
}

pub enum ExportMerge {
    Unchanged,
    BothChanged(Vec<AssetChange>),            // merged changeset
    Added(ExportSide),                        // non-conflicting add
    Removed,
    Conflict(ConflictKind),
}

pub enum ConflictKind {
    DeleteVsEdit { side_deleted: Side },
    ValueDivergence { path: String, ours: String, theirs: String, base: String },
    PinLinkedToDivergence { pin_id: [u8; 16], /* set deltas */ },
    ClassChanged { ours: String, theirs: String },
    UnparseableSide { side: Side },
}
```

Serialize this to the object store so the conflict survives re-invocation of
the merge tool (editor restart, etc.). A resolved merge writes a new .uasset
by replaying `ExportMerge::BothChanged` and user-picked values for each
`Conflict`.

## 7. Pin-connection conflict vs variable-rename conflict — contrast

**Pin connection conflict (local, within one export)**:
- Scope: a single `LinkedTo` set inside one K2Node export.
- Data needed: base set, ours set, theirs set of `(owning_node, pin_id)`.
- Resolution UI: three-way list with add/remove checkboxes — the user can
  almost always accept a union if the conflict is add-vs-add.

**Variable rename conflict (ripples across the asset)**:
- Scope: a `FieldDefinition` in a `BlueprintGeneratedClass` export; but the
  old name may be referenced by many `K2Node_VariableGet`/`VariableSet`
  nodes' `VariableReference.MemberName` tagged property elsewhere in the
  same asset.
- Data needed: the rename itself *plus* every node whose
  `VariableReference.MemberName` matches the old name.
- Resolution: pick a winning name, then *rewrite* every referencing node's
  `MemberName`. This cannot be done purely from the diff against a single
  export — it's an asset-wide transformation.
- Implication: variable renames should be surfaced as a distinct
  `AssetMerge` operation that produces a `Vec<PatchOp>` rather than as a
  plain `FieldRename` change. This is the single biggest reason not to
  treat "rename" as just "remove + add with same fields".

## 8. Open questions

- **UObject reference resolution across packages**: `DefaultObject` and
  `PinSubCategoryObject` are `FPackageIndex` values that index into the
  current asset's import table. A merge that picks imports from both sides
  may renumber indices; the merge writer must rebuild the import table and
  rewrite all `FPackageIndex` occurrences. This is out of scope for the
  first merge pass — require both sides to share an import table for now.
- **FText equality**: the current K2Node decoder skips FText contents. Two
  `DefaultTextValue` changes that disagree won't be detected by the
  structured diff — so "no diff" ≠ "mergeable" here. The merge engine
  should fall back to byte-equality on export blobs before declaring a
  clean merge.
- **Ordering**: pin iteration order is preserved in UE but not semantically
  meaningful for most pin kinds. If both sides reorder pins without other
  changes, treat as no-op.

## 9. Next implementation steps (out of scope for this task)

1. Introduce a `BaseSide` parameter to the diff engine to compute
   change-sets relative to a common ancestor (currently it's pairwise).
2. Expand `PinConnectionsChanged` into `PinLinkedToDelta { added, removed }`
   using the `PinRef` set.
3. Write `merge_assets(base, ours, theirs) -> Result<MergedAsset, Vec<Conflict>>`
   that composes the above and emits a new `.uasset` on success.
4. CLI: `forge merge <path>` that surfaces the conflict list and writes a
   merge-conflict marker file (since `.uasset` is binary, conflict markers
   need a sidecar `.forge-merge` file, not inline).
