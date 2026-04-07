// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{bail, Result};
use forge_core::uasset;

pub fn run(path: String, json: bool) -> Result<()> {
    let path = std::path::Path::new(&path);

    if !path.exists() {
        bail!("File not found: {}", path.display());
    }

    if !uasset::is_uasset_path(&path.to_string_lossy()) {
        bail!("Not a UE asset file (.uasset or .umap): {}", path.display());
    }

    let data = std::fs::read(path)?;
    let metadata = uasset::parse_uasset(&data)
        .ok_or_else(|| anyhow::anyhow!(
            "Failed to parse asset header (may be unsupported UE version): {}",
            path.display()
        ))?;

    if json {
        let output = serde_json::json!({
            "path": path.display().to_string(),
            "asset_class": metadata.asset_class,
            "engine_version": metadata.engine_version,
            "package_flags": metadata.package_flags,
            "dependencies": metadata.dependencies,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Asset:          {}", path.display());
        println!("Class:          {}", if metadata.asset_class.is_empty() { "Unknown" } else { &metadata.asset_class });
        println!("Engine:         {}", if metadata.engine_version.is_empty() { "Unknown" } else { &metadata.engine_version });

        if !metadata.package_flags.is_empty() {
            println!("Flags:          {}", metadata.package_flags.join(", "));
        }

        if !metadata.dependencies.is_empty() {
            println!("Dependencies:   ({} packages)", metadata.dependencies.len());
            for dep in &metadata.dependencies {
                println!("  - {}", dep);
            }
        }
    }

    Ok(())
}
