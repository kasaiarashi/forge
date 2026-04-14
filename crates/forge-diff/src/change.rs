//! `AssetChange` enum — the unit of semantic diff output emitted by handlers.

use std::fmt;
use forge_unreal::structured::ImportInfo;
use forge_unreal::ffield::FieldDefinition;

/// A single semantic change within a UE asset.
#[derive(Debug)]
pub enum AssetChange {
    ImportAdded(ImportInfo),
    ImportRemoved(ImportInfo),
    ExportAdded {
        name: String,
        class: String,
    },
    ExportRemoved {
        name: String,
        class: String,
    },
    PropertyChanged {
        export_name: String,
        property_path: String,
        old_value: String,
        new_value: String,
    },
    PropertyAdded {
        export_name: String,
        property_name: String,
        value: String,
    },
    PropertyRemoved {
        export_name: String,
        property_name: String,
        value: String,
    },
    ExportDataChanged {
        export_name: String,
        description: String,
    },
    /// An enumerator was added to a UserDefinedEnum.
    EnumValueAdded {
        export_name: String,
        value_name: String,
        display_name: Option<String>,
    },
    /// An enumerator was removed from a UserDefinedEnum.
    EnumValueRemoved {
        export_name: String,
        value_name: String,
    },
    /// A variable/property definition was added to a class/struct.
    FieldAdded {
        export_name: String,
        field: FieldDefinition,
    },
    /// A variable/property definition was removed from a class/struct.
    FieldRemoved {
        export_name: String,
        field: FieldDefinition,
    },
    /// A Blueprint pin was added to a K2Node.
    PinAdded {
        export_name: String,
        pin_name: String,
        pin_category: String,
        default_value: Option<String>,
    },
    /// A Blueprint pin was removed from a K2Node.
    PinRemoved {
        export_name: String,
        pin_name: String,
        pin_category: String,
    },
    /// A Blueprint pin was renamed (matched by stable PinId GUID).
    PinRenamed {
        export_name: String,
        old_name: String,
        new_name: String,
    },
    /// A Blueprint pin's category (type) changed.
    PinTypeChanged {
        export_name: String,
        pin_name: String,
        old_category: String,
        new_category: String,
    },
    /// A Blueprint pin's default value changed.
    PinDefaultChanged {
        export_name: String,
        pin_name: String,
        old_value: String,
        new_value: String,
    },
    /// A Blueprint pin's connection count changed (wires added/removed).
    PinConnectionsChanged {
        export_name: String,
        pin_name: String,
        old_count: usize,
        new_count: usize,
    },
}

impl fmt::Display for AssetChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetChange::ImportAdded(imp) => {
                write!(f, "  + import: {}", imp.object_name)
            }
            AssetChange::ImportRemoved(imp) => {
                write!(f, "  - import: {}", imp.object_name)
            }
            AssetChange::ExportAdded { name, class } => {
                write!(f, "  + {} ({})", name, class)
            }
            AssetChange::ExportRemoved { name, class } => {
                write!(f, "  - {} ({})", name, class)
            }
            AssetChange::PropertyChanged {
                export_name,
                property_path,
                old_value,
                new_value,
            } => {
                write!(
                    f,
                    "  [{}] ~ {}: {} -> {}",
                    export_name, property_path, old_value, new_value
                )
            }
            AssetChange::PropertyAdded {
                export_name,
                property_name,
                value,
            } => {
                write!(f, "  [{}] + {}: {}", export_name, property_name, value)
            }
            AssetChange::PropertyRemoved {
                export_name,
                property_name,
                value,
            } => {
                write!(f, "  [{}] - {}: {}", export_name, property_name, value)
            }
            AssetChange::ExportDataChanged {
                export_name,
                description,
            } => {
                write!(f, "  [{}] ~ {}", export_name, description)
            }
            AssetChange::EnumValueAdded { export_name, value_name, display_name } => {
                if let Some(dn) = display_name {
                    write!(f, "  [{}] + enum: {} ({})", export_name, value_name, dn)
                } else {
                    write!(f, "  [{}] + enum: {}", export_name, value_name)
                }
            }
            AssetChange::EnumValueRemoved { export_name, value_name } => {
                write!(f, "  [{}] - enum: {}", export_name, value_name)
            }
            AssetChange::FieldAdded { export_name, field } => {
                write!(f, "  [{}] + variable: {}", export_name, field)
            }
            AssetChange::FieldRemoved { export_name, field } => {
                write!(f, "  [{}] - variable: {}", export_name, field)
            }
            AssetChange::PinAdded { export_name, pin_name, pin_category, default_value } => {
                match default_value {
                    Some(dv) if !dv.is_empty() => write!(
                        f, "  [{}] + pin \"{}\" ({}) default: {}",
                        export_name, pin_name, pin_category, dv
                    ),
                    _ => write!(f, "  [{}] + pin \"{}\" ({}, no default)",
                        export_name, pin_name, pin_category),
                }
            }
            AssetChange::PinRemoved { export_name, pin_name, pin_category } => {
                write!(f, "  [{}] - pin \"{}\" ({})", export_name, pin_name, pin_category)
            }
            AssetChange::PinRenamed { export_name, old_name, new_name } => {
                write!(f, "  [{}] ~ pin renamed: \"{}\" -> \"{}\"", export_name, old_name, new_name)
            }
            AssetChange::PinTypeChanged { export_name, pin_name, old_category, new_category } => {
                write!(f, "  [{}] ~ pin \"{}\" type: {} -> {}",
                    export_name, pin_name, old_category, new_category)
            }
            AssetChange::PinDefaultChanged { export_name, pin_name, old_value, new_value } => {
                write!(f, "  [{}] ~ pin \"{}\" default: {} -> {}",
                    export_name, pin_name, old_value, new_value)
            }
            AssetChange::PinConnectionsChanged { export_name, pin_name, old_count, new_count } => {
                write!(f, "  [{}] ~ pin \"{}\" connections: {} -> {}",
                    export_name, pin_name, old_count, new_count)
            }
        }
    }
}
