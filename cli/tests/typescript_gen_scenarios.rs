//! Integration tests for TypeScript code generation scenarios.
//!
//! These tests verify the file structure and content of generated TypeScript code
//! for three main scenarios:
//! 1. Local service only
//! 2. Remote service only
//! 3. Both local and remote services
//!
//! Run with:
//! `cargo test --test typescript_gen_scenarios -- --test-threads=1`

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use tempfile::TempDir;

fn actr_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_actr"))
}

fn framework_codegen_typescript_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("actr-cli should live under the workspace root")
        .join("tools/protoc-gen/typescript")
}

fn prepare_typescript_codegen_tools() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = framework_codegen_typescript_dir();

        // Ensure we are in the right directory
        assert!(
            dir.exists(),
            "tools/protoc-gen/typescript dir not found at {}",
            dir.display()
        );

        println!("Running npm install in {}...", dir.display());
        let npm_install = Command::new("npm")
            .args(["install"])
            .current_dir(&dir)
            .output()
            .expect("failed to run npm install");
        if !npm_install.status.success() {
            panic!(
                "npm install failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&npm_install.stdout),
                String::from_utf8_lossy(&npm_install.stderr)
            );
        }

        println!("Running npm run bundle in {}...", dir.display());
        let npm_bundle = Command::new("npm")
            .args(["run", "bundle"])
            .current_dir(&dir)
            .output()
            .expect("failed to run npm run bundle");
        if !npm_bundle.status.success() {
            panic!(
                "npm bundle failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&npm_bundle.stdout),
                String::from_utf8_lossy(&npm_bundle.stderr)
            );
        }

        dir
    })
}

fn run_actr(args: &[&str], cwd: &Path) -> Output {
    let tool_dir = prepare_typescript_codegen_tools();

    // Construct PATH to include the local plugin and its dependencies
    let mut path_entries = vec![tool_dir.join("scripts"), tool_dir.join("node_modules/.bin")];
    if let Some(existing) = std::env::var_os("PATH") {
        path_entries.extend(std::env::split_paths(&existing));
    }
    let path = std::env::join_paths(path_entries).expect("failed to construct PATH");

    Command::new(actr_bin())
        .args(args)
        .current_dir(cwd)
        .env("PATH", path)
        .output()
        .expect("failed to run actr binary")
}

fn assert_success(out: &Output, context: &str) {
    if !out.status.success() {
        panic!(
            "{context} failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

// Project construction helpers

fn write_ts_project_files(root: &Path) {
    fs::write(
        root.join("package.json"),
        r#"{
  "name": "test-gen",
  "dependencies": {
    "@actrium/actr": "*"
  }
}"#,
    )
    .unwrap();

    fs::write(
        root.join("tsconfig.json"),
        r#"{
  "compilerOptions": {
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "node",
    "strict": true,
    "skipLibCheck": true
  }
}"#,
    )
    .unwrap();
}

fn write_actr_toml_local_only(root: &Path) {
    fs::write(
        root.join("actr.toml"),
        r#"edition = 1
[package]
name = "LocalService"
manufacturer = "acme"
version="0.0.1"
[dependencies]

[system.signaling]
url = "wss://localhost:8080"

[system.ais_endpoint]
url = "https://localhost:8080/ais"

[system.deployment]
realm_id = 1
"#,
    )
    .unwrap();
}

fn write_actr_toml_with_remote(root: &Path) {
    fs::write(
        root.join("actr.toml"),
        r#"edition = 1
[package]
name = "RemoteApp"
manufacturer = "acme"
version="0.0.1"
[dependencies]
echo-service = { actr_type = "acme:EchoService:0.0.1" }

[system.signaling]
url = "wss://localhost:8080"

[system.ais_endpoint]
url = "https://localhost:8080/ais"

[system.deployment]
realm_id = 1
"#,
    )
    .unwrap();
}

fn write_actr_toml_both(root: &Path) {
    fs::write(
        root.join("actr.toml"),
        r#"edition = 1
[package]
name = "BothService"
manufacturer = "acme"
version = "0.0.1"
[dependencies]
echo-service = { actr_type = "acme:EchoService:0.0.1" }

[system.signaling]
url = "wss://localhost:8080"

[system.ais_endpoint]
url = "https://localhost:8080/ais"

[system.deployment]
realm_id = 1
"#,
    )
    .unwrap();
}

fn write_actr_toml_both_with_two_remotes(root: &Path) {
    fs::write(
        root.join("actr.toml"),
        r#"edition = 1
[package]
name = "BothTwoRemotesService"
manufacturer = "acme"
version="0.0.1"
[dependencies]
echo-service = { actr_type = "acme:EchoService:0.0.1" }
profile-service = { actr_type = "acme:ProfileService:0.0.1" }

[system.signaling]
url = "wss://localhost:8080"

[system.ais_endpoint]
url = "https://localhost:8080/ais"

[system.deployment]
realm_id = 1
"#,
    )
    .unwrap();
}

fn write_lock_file_empty(root: &Path) {
    fs::write(
        root.join("Actr.lock.toml"),
        r#"[metadata]
version = 1
generated_at = "2026-03-03T00:00:00Z"
"#,
    )
    .unwrap();
}

fn write_lock_file_with_echo(root: &Path) {
    fs::write(
        root.join("Actr.lock.toml"),
        r#"[metadata]
version = 1
generated_at = "2026-03-03T00:00:00Z"

[[dependency]]
name = "echo-service"
actr_type = "acme:EchoService:1.0.0"
fingerprint = "service_semantic:123"
cached_at = "2026-03-03T00:00:00Z"
files = [
    { path = "echo-service/echo.proto", fingerprint = "semantic:456" }
]
"#,
    )
    .unwrap();
}

fn write_lock_file_with_echo_and_profile(root: &Path) {
    fs::write(
        root.join("Actr.lock.toml"),
        r#"[metadata]
version = 1
generated_at = "2026-03-03T00:00:00Z"

[[dependency]]
name = "echo-service"
actr_type = "acme:EchoService:1.0.0"
fingerprint = "service_semantic:123"
cached_at = "2026-03-03T00:00:00Z"
files = [
    { path = "echo-service/echo.proto", fingerprint = "semantic:456" }
]

[[dependency]]
name = "profile-service"
actr_type = "acme:ProfileService:1.0.0"
fingerprint = "service_semantic:789"
cached_at = "2026-03-03T00:00:00Z"
files = [
    { path = "profile-service/profile.proto", fingerprint = "semantic:999" }
]
"#,
    )
    .unwrap();
}

fn write_local_greeter_proto(root: &Path) {
    let proto_dir = root.join("protos");
    fs::create_dir_all(&proto_dir).unwrap();
    fs::write(
        proto_dir.join("greeter.proto"),
        r#"syntax = "proto3";
package greeter;

message HelloRequest {
  string name = 1;
}

message HelloResponse {
  string message = 1;
}

service Greeter {
  rpc SayHello(HelloRequest) returns (HelloResponse);
}
"#,
    )
    .unwrap();
}

fn write_remote_echo_proto(root: &Path) {
    let remote_proto_dir = root.join("protos/remote/echo-service");
    fs::create_dir_all(&remote_proto_dir).unwrap();
    fs::write(
        remote_proto_dir.join("echo.proto"),
        r#"syntax = "proto3";
package echo;

message EchoRequest {
  string message = 1;
}

message EchoResponse {
  string message = 1;
}

service EchoService {
  rpc Echo(EchoRequest) returns (EchoResponse);
}
"#,
    )
    .unwrap();
}

fn write_remote_profile_proto(root: &Path) {
    let remote_proto_dir = root.join("protos/remote/profile-service");
    fs::create_dir_all(&remote_proto_dir).unwrap();
    fs::write(
        remote_proto_dir.join("profile.proto"),
        r#"syntax = "proto3";
package profile;

message GetProfileRequest {
  string user_id = 1;
}

message GetProfileResponse {
  string nickname = 1;
}

service ProfileService {
  rpc GetProfile(GetProfileRequest) returns (GetProfileResponse);
}
"#,
    )
    .unwrap();
}

#[test]
fn test_local_service_only() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_ts_project_files(root);
    write_actr_toml_local_only(root);
    write_lock_file_empty(root);
    write_local_greeter_proto(root);

    let out = run_actr(&["gen", "-l", "typescript"], root);
    assert_success(&out, "actr gen -l typescript (local only)");

    let gen_dir = root.join("src/generated");
    assert!(gen_dir.join("greeter_pb.ts").exists());
    assert!(!gen_dir.join("greeter_client.ts").exists());

    // Local services are unified into local_actor.ts
    assert!(
        gen_dir.join("local_actor.ts").exists(),
        "local_actor.ts should exist for local-only projects"
    );

    // Verify directory flattening
    assert!(
        !gen_dir.join("local").exists(),
        "local/ directory should have been flattened"
    );
    assert!(
        !gen_dir.join("remote").exists(),
        "remote/ directory should not exist"
    );

    let actor_content = fs::read_to_string(gen_dir.join("local_actor.ts")).unwrap();
    assert!(actor_content.contains("export const HelloRequest = {"));
    assert!(actor_content.contains("case HelloRequest.routeKey"));
    assert!(actor_content.contains("HelloRequest.routeKey"));
    assert!(actor_content.contains("HelloRequest.decode(envelope.payload)"));
    assert!(actor_content.contains("export type LocalHandlers"));
    assert!(actor_content.contains("default:"));
    assert!(actor_content.contains("Unknown route"));
    assert!(!actor_content.contains("export const ROUTES"));
    assert!(!actor_content.contains("ROUTES.find("));

    let actr_service_content = fs::read_to_string(root.join("src/actr_service.ts")).unwrap();
    assert!(actr_service_content.contains("from './generated/local_actor'"));
    assert!(actr_service_content.contains("dispatchLocalActor(ctx, envelope, handlers)"));
    assert!(actr_service_content.contains("[HelloRequest.routeKey]: HelloRequest.decode"));
}

#[test]
fn test_remote_service_only() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_ts_project_files(root);
    write_actr_toml_with_remote(root);
    write_lock_file_with_echo(root);
    write_remote_echo_proto(root);

    let out = run_actr(&["gen", "-l", "typescript"], root);
    assert_success(&out, "actr gen -l typescript (remote only)");

    let gen_dir = root.join("src/generated");
    // Remote files are lifted: remote/echo-service/echo.proto -> src/generated/echo-service/echo_pb.ts
    assert!(gen_dir.join("echo-service/echo_pb.ts").exists());
    assert!(gen_dir.join("echo-service/echo_client.ts").exists());

    // When remote services exist, local_actor.ts MUST be generated
    assert!(gen_dir.join("local_actor.ts").exists());

    // Verify directory lifting
    assert!(
        !gen_dir.join("remote").exists(),
        "remote/ directory should have been lifted"
    );

    let actor_content = fs::read_to_string(gen_dir.join("local_actor.ts")).unwrap();
    assert!(actor_content.contains("switch (envelope.routeKey)"));
    assert!(actor_content.contains("case EchoRequest.routeKey"));
    assert!(actor_content.contains("EchoRequest.routeKey"));
    assert!(actor_content.contains("dispatchLocalActor"));
    assert!(
        actor_content.contains("ctx.discover({ manufacturer: \"acme\", name: \"EchoService\" })")
    );
    assert!(actor_content.contains("export type LocalHandlers = {}"));
    assert!(actor_content.contains("default:"));
    assert!(actor_content.contains("Unknown route"));
    assert!(!actor_content.contains("export const ROUTES"));
    assert!(!actor_content.contains("ROUTES.find("));
    assert!(!actor_content.contains("HelloRequest.routeKey"));

    let actr_service_content = fs::read_to_string(root.join("src/actr_service.ts")).unwrap();
    assert!(actr_service_content.contains("const handlers: LocalHandlers = {};"));
    assert!(actr_service_content.contains("EchoRequest.routeKey"));
    assert!(actr_service_content.contains("EchoRequest.encode"));
    assert!(actr_service_content.contains("EchoRequest.response.decode"));
    assert!(!actr_service_content.contains("HelloRequest.decode"));
}

#[test]
fn test_local_and_remote_services() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_ts_project_files(root);
    write_actr_toml_both(root);
    write_lock_file_with_echo(root);
    write_local_greeter_proto(root);
    write_remote_echo_proto(root);

    let out = run_actr(&["gen", "-l", "typescript"], root);
    assert_success(&out, "actr gen -l typescript (both)");

    let gen_dir = root.join("src/generated");

    // Local files
    assert!(gen_dir.join("greeter_pb.ts").exists());
    assert!(!gen_dir.join("greeter_client.ts").exists());

    // Remote files
    assert!(gen_dir.join("echo-service/echo_pb.ts").exists());
    assert!(gen_dir.join("echo-service/echo_client.ts").exists());

    // local_actor.ts
    assert!(gen_dir.join("local_actor.ts").exists());

    let actor_content = fs::read_to_string(gen_dir.join("local_actor.ts")).unwrap();

    // Should contain both local and remote service routes
    assert!(actor_content.contains("case HelloRequest.routeKey"));
    assert!(actor_content.contains("case EchoRequest.routeKey"));
    assert!(actor_content.contains("HelloRequest.routeKey"));
    assert!(actor_content.contains("EchoRequest.routeKey"));
    assert!(
        actor_content.contains("ctx.discover({ manufacturer: \"acme\", name: \"EchoService\" })")
    );
    assert!(actor_content.contains("default:"));
    assert!(actor_content.contains("Unknown route"));
    assert!(!actor_content.contains("export const ROUTES"));
    assert!(!actor_content.contains("ROUTES.find("));

    // Check import path for lifted remote file (should not contain "remote/")
    assert!(actor_content.contains("./echo-service/echo_client"));
    assert!(!actor_content.contains("./remote/"));

    let actr_service_content = fs::read_to_string(root.join("src/actr_service.ts")).unwrap();
    assert!(actr_service_content.contains("from './generated/local_actor'"));
    assert!(actr_service_content.contains("async handleSayHello(request, _ctx)"));
    assert!(actr_service_content.contains("from './generated/echo-service/echo_client';"));
    assert!(actr_service_content.contains("[HelloRequest.routeKey]: HelloRequest.decode"));
    assert!(actr_service_content.contains("EchoRequest.routeKey"));
}

#[test]
fn test_remote_dispatch_uses_switch_cases_instead_of_routes_table() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_ts_project_files(root);
    write_actr_toml_both_with_two_remotes(root);
    write_lock_file_with_echo_and_profile(root);
    write_local_greeter_proto(root);
    write_remote_echo_proto(root);
    write_remote_profile_proto(root);

    let out = run_actr(&["gen", "-l", "typescript"], root);
    assert_success(&out, "actr gen -l typescript (local + two remotes)");

    let gen_dir = root.join("src/generated");
    let actor_content = fs::read_to_string(gen_dir.join("local_actor.ts")).unwrap();

    assert!(actor_content.contains("case HelloRequest.routeKey"));
    assert!(actor_content.contains("case EchoRequest.routeKey"));
    assert!(actor_content.contains("case GetProfileRequest.routeKey"));
    assert!(
        actor_content.contains("ctx.discover({ manufacturer: \"acme\", name: \"EchoService\" })")
    );
    assert!(
        actor_content
            .contains("ctx.discover({ manufacturer: \"acme\", name: \"ProfileService\" })")
    );
    assert!(actor_content.contains("default:"));
    assert!(actor_content.contains("Unknown route"));
    assert!(!actor_content.contains("export const ROUTES"));
    assert!(!actor_content.contains("ROUTES.find("));
}
