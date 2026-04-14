//! Import-table diff — additions and removals keyed by `object_name`.

use std::collections::BTreeMap;
use forge_unreal::structured::ImportInfo;

use super::{AssetDiffHandler, DiffContext};
use crate::change::AssetChange;

pub struct ImportHandler;

impl AssetDiffHandler for ImportHandler {
    fn name(&self) -> &'static str { "imports" }

    fn diff_top_level(&self, ctx: &DiffContext<'_>, sink: &mut Vec<AssetChange>) {
        diff_imports(&ctx.old.asset.imports, &ctx.new.asset.imports, sink);
    }
}

fn diff_imports(old: &[ImportInfo], new: &[ImportInfo], changes: &mut Vec<AssetChange>) {
    let old_map: BTreeMap<&str, &ImportInfo> =
        old.iter().map(|i| (i.object_name.as_str(), i)).collect();
    let new_map: BTreeMap<&str, &ImportInfo> =
        new.iter().map(|i| (i.object_name.as_str(), i)).collect();

    for (name, imp) in &old_map {
        if !new_map.contains_key(name) {
            changes.push(AssetChange::ImportRemoved((*imp).clone()));
        }
    }

    for (name, imp) in &new_map {
        if !old_map.contains_key(name) {
            changes.push(AssetChange::ImportAdded((*imp).clone()));
        }
    }
}
