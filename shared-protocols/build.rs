//! Build script for generating Rust code from Protobuf files

use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Configure tonic-build to generate actor framework adapter code
    tonic_build::configure()
        .build_server(false) // We're not building gRPC servers
        .build_client(false) // We're not building gRPC clients
        .protoc_arg("--plugin=protoc-gen-actorframework=../target/debug/protoc-gen-actorframework")
        .protoc_arg("--actorframework_out=src")
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .out_dir(&out_dir)
        .compile(
            &[
                "../proto/webrtc.proto",
                "../proto/actor.proto",
                "../proto/signaling.proto",
                "../proto/echo.proto",
                "../proto/media_streaming.proto",
                "../proto/file_transfer.proto",
                "../proto/stream_test.proto",
            ],
            &["../proto"],
        )?;

    // Tell cargo to re-run if any proto files change
    println!("cargo:rerun-if-changed=../proto");
    println!("cargo:rerun-if-changed=../target/debug/protoc-gen-actorframework");

    // Try to run rustfmt on generated files in src/
    let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let src_dir = crate_dir.join("src");
    if let Ok(entries) = std::fs::read_dir(&src_dir) {
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "rs" {
                    files.push(path);
                }
            }
        }
        if !files.is_empty() {
            let mut cmd = std::process::Command::new("rustfmt");
            cmd.arg("--edition").arg("2021");
            for f in &files {
                cmd.arg(f);
            }
            let _ = cmd.status();
        }
    }

    Ok(())
}
