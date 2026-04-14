fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        // Add serde derives to every generated message so the web UI can
        // serialize them straight to JSON without re-wrapping in DTOs.
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .type_attribute(".", "#[serde(rename_all = \"snake_case\")]")
        .compile_protos(&["../../proto/forge.proto"], &["../../proto"])?;
    Ok(())
}
