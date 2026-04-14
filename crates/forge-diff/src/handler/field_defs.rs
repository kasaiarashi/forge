//! Field-definition diff — class/struct variable definition adds/removes.

use std::collections::BTreeMap;
use forge_unreal::ffield::FieldDefinition;
use forge_unreal::structured::ExportInfo;

use super::{AssetDiffHandler, DiffContext};
use crate::change::AssetChange;

pub struct FieldDefHandler;

impl AssetDiffHandler for FieldDefHandler {
    fn name(&self) -> &'static str { "field_defs" }

    fn diff_matched_export(
        &self,
        _ctx: &DiffContext<'_>,
        old_exp: &ExportInfo,
        new_exp: &ExportInfo,
        sink: &mut Vec<AssetChange>,
    ) -> bool {
        diff_field_definitions(
            new_exp.object_name.as_str(),
            &old_exp.field_definitions,
            &new_exp.field_definitions,
            sink,
        );
        false
    }
}

fn diff_field_definitions(
    export_name: &str,
    old_fields: &Option<Vec<FieldDefinition>>,
    new_fields: &Option<Vec<FieldDefinition>>,
    changes: &mut Vec<AssetChange>,
) {
    let (old_f, new_f) = match (old_fields, new_fields) {
        (Some(o), Some(n)) => (o.as_slice(), n.as_slice()),
        (None, Some(n)) => {
            for f in n {
                changes.push(AssetChange::FieldAdded {
                    export_name: export_name.to_string(),
                    field: f.clone(),
                });
            }
            return;
        }
        (Some(o), None) => {
            for f in o {
                changes.push(AssetChange::FieldRemoved {
                    export_name: export_name.to_string(),
                    field: f.clone(),
                });
            }
            return;
        }
        (None, None) => return,
    };

    let old_map: BTreeMap<&str, &FieldDefinition> = old_f.iter()
        .map(|f| (f.field_name.as_str(), f))
        .collect();
    let new_map: BTreeMap<&str, &FieldDefinition> = new_f.iter()
        .map(|f| (f.field_name.as_str(), f))
        .collect();

    for (name, field) in &old_map {
        if !new_map.contains_key(name) {
            changes.push(AssetChange::FieldRemoved {
                export_name: export_name.to_string(),
                field: (*field).clone(),
            });
        }
    }

    for (name, field) in &new_map {
        if !old_map.contains_key(name) {
            changes.push(AssetChange::FieldAdded {
                export_name: export_name.to_string(),
                field: (*field).clone(),
            });
        }
    }
}
