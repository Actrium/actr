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

    Ok(())
}
