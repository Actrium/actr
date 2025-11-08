use std::path::PathBuf;

use glob::glob;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let generated_dir = PathBuf::from("src/generated");
    std::fs::create_dir_all(&generated_dir)?;

    // Collect all .proto files using glob
    let proto_files: Vec<String> = glob("proto/**/*.proto")?
        .filter_map(Result::ok)
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    if proto_files.is_empty() {
        return Err("No proto files found".into());
    }

    // Configure prost-build for proto2 files
    prost_build::Config::new()
        .out_dir(&generated_dir)
        // Use bytes::Bytes instead of Vec<u8> for better zero-copy performance
        .bytes(["."])
        // Add serde and Ord/PartialOrd for core identity types
        // (used in HashMaps, BTreeMaps, and serialized outside of protobuf)
        .type_attribute(
            "ActrId",
            "#[derive(Ord, PartialOrd, serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "ActrType",
            "#[derive(Ord, PartialOrd, serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "Realm",
            "#[derive(Ord, PartialOrd, serde::Serialize, serde::Deserialize)]",
        )
        .compile_protos(&proto_files, &["proto/"])?;

    // Tell cargo to rerun if proto files change
    println!("cargo:rerun-if-changed=proto/");

    Ok(())
}
