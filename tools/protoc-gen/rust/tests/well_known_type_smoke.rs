use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn protoc_passes_well_known_type_descriptors_to_the_plugin() {
    let temp = TempDir::new().expect("create temp directory");
    let proto_root = temp.path().join("proto");
    let local_dir = proto_root.join("local");
    let well_known_dir = proto_root.join("google/protobuf");
    let output_dir = temp.path().join("generated");
    fs::create_dir_all(&local_dir).expect("create local proto directory");
    fs::create_dir_all(&well_known_dir).expect("create well-known proto directory");
    fs::create_dir_all(&output_dir).expect("create generated directory");

    fs::write(
        well_known_dir.join("empty.proto"),
        r#"syntax = "proto3";
package google.protobuf;
message Empty {}
"#,
    )
    .expect("write empty.proto");
    fs::write(
        local_dir.join("client.proto"),
        r#"syntax = "proto3";
package client;
import "google/protobuf/empty.proto";
service Client {
  rpc Ping(google.protobuf.Empty) returns (google.protobuf.Empty);
}
"#,
    )
    .expect("write client.proto");

    let output = Command::new("protoc")
        .arg(format!("--proto_path={}", proto_root.display()))
        .arg(format!(
            "--plugin=protoc-gen-actrframework={}",
            env!("CARGO_BIN_EXE_protoc-gen-actrframework")
        ))
        .arg("--actrframework_opt=manufacturer=acme,LocalFiles=local/client.proto")
        .arg(format!("--actrframework_out={}", output_dir.display()))
        .arg("local/client.proto")
        .output()
        .expect("run protoc");

    assert!(
        output.status.success(),
        "protoc failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output_dir.join("client_actor.rs").is_file(),
        "framework code was not generated"
    );

    let metadata: Value = serde_json::from_str(
        &fs::read_to_string(output_dir.join("actr-gen-meta.json"))
            .expect("read generated metadata"),
    )
    .expect("parse generated metadata");
    let method = &metadata["local_services"][0]["methods"][0];

    assert_eq!(method["input_ref"]["proto_type"], "google.protobuf.Empty");
    assert_eq!(
        method["input_ref"]["proto_file"],
        "google/protobuf/empty.proto"
    );
    assert_eq!(method["output_ref"]["proto_type"], "google.protobuf.Empty");
}
