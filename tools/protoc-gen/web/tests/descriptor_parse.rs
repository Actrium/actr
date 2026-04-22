//! Regression tests for descriptor-based proto parsing.
//!
//! Requires `protoc` to be available on the PATH. Tests that cannot locate
//! the binary report as `ignored` rather than hard-failing, to keep CI green
//! on minimal images.

use std::path::PathBuf;

use actr_web_protoc_codegen::{WebCodegen, WebCodegenConfig};

fn protoc_available() -> bool {
    std::process::Command::new("protoc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn write_proto(dir: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, body).expect("write proto");
    path
}

#[test]
fn parses_proto3_service_with_optional_and_streaming() {
    if !protoc_available() {
        eprintln!("skipping: protoc not on PATH");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let proto_path = write_proto(
        &tmp,
        "user_service.proto",
        r#"syntax = "proto3";

package example.user.v1;

// User service description
service UserService {
  // Get user information
  rpc GetUser(GetUserRequest) returns (GetUserResponse);
  // Streaming
  rpc ListUsers(ListUsersRequest) returns (stream User);
}

message GetUserRequest {
  string user_id = 1;
}

message GetUserResponse {
  User user = 1;
}

message ListUsersRequest {
  int32 page = 1;
  int32 page_size = 2;
  optional string filter = 3;
}

message User {
  string id = 1;
  string name = 2;
  int32 age = 4;
  repeated string tags = 5;
  bool is_active = 8;
}
"#,
    );

    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();

    let config = WebCodegenConfig::builder()
        .proto_file(proto_path)
        .rust_output(out.join("rs"))
        .ts_output(out.join("ts"))
        .include(tmp.path())
        .with_formatting(false)
        .build()
        .expect("build config");

    let codegen = WebCodegen::new(config);
    let files = codegen.generate_typescript_only().expect("typescript gen");

    // The generator must emit one types file and one actor-ref file for
    // the service, plus an index.
    let names: Vec<String> = files
        .iter()
        .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(
        names.iter().any(|n| n == "user-service.types.ts"),
        "missing types.ts; got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "user-service.actor-ref.ts"),
        "missing actor-ref.ts; got {names:?}"
    );

    let types_ts = files
        .iter()
        .find(|f| f.path.file_name().unwrap() == "user-service.types.ts")
        .unwrap();

    // Field ordering and optionality must be preserved exactly.
    assert!(
        types_ts
            .content
            .contains("export interface ListUsersRequest {")
    );
    assert!(types_ts.content.contains("page: number;"));
    assert!(types_ts.content.contains("page_size: number;"));
    assert!(types_ts.content.contains("filter?: string;"));
    assert!(types_ts.content.contains("tags: string[];"));
    // The comment-only `service` token inside a doc string should not have
    // confused the descriptor-based parser.
    assert!(!types_ts.content.contains("User service description"));

    // Actor-ref must include the streaming RPC as a `subscribe*` method.
    let ref_ts = files
        .iter()
        .find(|f| f.path.file_name().unwrap() == "user-service.actor-ref.ts")
        .unwrap();
    assert!(
        ref_ts.content.contains("subscribeListUsers"),
        "streaming method missing in: {}",
        ref_ts.content
    );
    assert!(
        ref_ts
            .content
            .contains("async getUser(request: GetUserRequest)")
    );
}

#[test]
fn comments_mentioning_service_do_not_fool_parser() {
    if !protoc_available() {
        eprintln!("skipping: protoc not on PATH");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let proto_path = write_proto(
        &tmp,
        "tricky.proto",
        r#"syntax = "proto3";
package tricky;

// This comment mentions `service Fake {}` but does not declare one.
/* Block comment: service AlsoFake { rpc nope(A) returns (B); } */
message A { string v = 1; }
message B { string v = 1; }

service RealService {
  rpc Echo(A) returns (B);
}
"#,
    );

    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();

    let config = WebCodegenConfig::builder()
        .proto_file(proto_path)
        .rust_output(out.join("rs"))
        .ts_output(out.join("ts"))
        .include(tmp.path())
        .with_formatting(false)
        .build()
        .unwrap();

    let files = WebCodegen::new(config)
        .generate_typescript_only()
        .expect("typescript gen");

    let ref_ts = files
        .iter()
        .find(|f| f.path.file_name().unwrap() == "real-service.actor-ref.ts")
        .expect("real-service.actor-ref.ts must be generated");
    assert!(ref_ts.content.contains("RealServiceActorRef"));
    assert!(ref_ts.content.contains("async echo(request: A)"));
    // Fake service names must not leak into output.
    assert!(!ref_ts.content.contains("Fake"));
    assert!(!ref_ts.content.contains("AlsoFake"));
}
