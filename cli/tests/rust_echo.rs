//! Integration tests for `actr init -l rust --template echo`
//!
//! These tests verify scaffold generation only (fast).
//! For end-to-end tests with real service communication, see `e2e_rust_echo.rs`.
//!
//! Run with: `cargo test --test rust_echo`

use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn actr_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_actr"))
}

fn run_actr(args: &[&str], cwd: &std::path::Path) -> Output {
    Command::new(actr_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run actr binary")
}

fn assert_actr_success(out: &Output, context: &str) {
    assert!(
        out.status.success(),
        "{context} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

fn init_rust_echo_app(parent: &std::path::Path, name: &str) -> std::path::PathBuf {
    let out = run_actr(
        &[
            "init",
            "-l",
            "rust",
            "--template",
            "echo",
            "--role",
            "app",
            "--signaling",
            "wss://actrix1.develenv.com",
            "--manufacturer",
            "acme",
            name,
        ],
        parent,
    );
    assert_actr_success(&out, "actr init (app)");
    parent.join(name)
}

fn init_rust_echo_service(parent: &std::path::Path, name: &str) -> std::path::PathBuf {
    let out = run_actr(
        &[
            "init",
            "-l",
            "rust",
            "--template",
            "echo",
            "--role",
            "service",
            "--signaling",
            "wss://actrix1.develenv.com",
            "--manufacturer",
            "acme",
            name,
        ],
        parent,
    );
    assert_actr_success(&out, "actr init (service)");
    parent.join(name)
}

fn init_rust_echo_both(parent: &std::path::Path, name: &str) -> std::path::PathBuf {
    let out = run_actr(
        &[
            "init",
            "-l",
            "rust",
            "--template",
            "echo",
            "--role",
            "both",
            "--signaling",
            "wss://actrix1.develenv.com",
            "--manufacturer",
            "example",
            name,
        ],
        parent,
    );
    assert_actr_success(&out, "actr init (both)");
    parent.join(name)
}

// ---------------------------------------------------------------------------
// App role: scaffold validation
// ---------------------------------------------------------------------------

/// Verify all generated files and their content for an app-role project.
#[test]
fn rust_echo_app_scaffold() {
    let tmp = TempDir::new().unwrap();
    let dir = init_rust_echo_app(tmp.path(), "my-echo-app");

    // -- files exist --
    for path in &[
        "Cargo.toml",
        "actr.toml",
        "src/main.rs",
        "README.md",
        "protos/local/local.proto",
        ".protoc-plugin.toml",
    ] {
        assert!(dir.join(path).exists(), "{path} should exist");
    }

    // -- Cargo.toml --
    let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    assert!(cargo.contains(r#"name = "my-echo-app""#), "package name");
    assert!(cargo.contains(r#"edition = "2024""#), "edition");
    assert!(
        cargo.contains("actr = "),
        "missing dependency actr in Cargo.toml"
    );

    // -- actr.toml --
    let actr = std::fs::read_to_string(dir.join("actr.toml")).unwrap();
    assert!(
        actr.contains("wss://actrix1.develenv.com/signaling/ws"),
        "signaling URL"
    );

    // -- local.proto should contain the generated bridge service --
    let proto = std::fs::read_to_string(dir.join("protos/local/local.proto")).unwrap();
    assert!(
        proto.contains("service MyEchoAppClientApp {}"),
        "app local.proto must define the bridge service"
    );
    assert!(
        !dir.join("src/echo_app.rs").exists(),
        "app scaffold should no longer generate src/echo_app.rs"
    );

    // -- main.rs --
    let main = std::fs::read_to_string(dir.join("src/main.rs")).unwrap();
    assert!(
        !main.contains("mod echo_app"),
        "main.rs should not declare mod echo_app"
    );
    assert!(
        main.contains("attach_none(config)"),
        "main.rs should start a client-only node"
    );
    assert!(
        !main.contains("MyEchoAppClientAppHandler"),
        "main.rs should not depend on generated handler traits"
    );
    assert!(
        !main.contains("MyEchoAppClientAppWorkload"),
        "main.rs should not depend on generated workload types"
    );
}

// ---------------------------------------------------------------------------
// Service role: scaffold validation
// ---------------------------------------------------------------------------

/// Verify all generated files and their content for a service-role project.
#[test]
fn rust_echo_service_scaffold() {
    let tmp = TempDir::new().unwrap();
    let dir = init_rust_echo_service(tmp.path(), "my-echo-svc");

    // -- files exist --
    for path in &[
        "Cargo.toml",
        "actr.toml",
        "src/main.rs",
        "src/echo_service.rs",
    ] {
        assert!(dir.join(path).exists(), "{path} should exist");
    }

    // -- actr.toml --
    let actr = std::fs::read_to_string(dir.join("actr.toml")).unwrap();
    assert!(
        actr.contains(r#"exports = ["protos/local/echo.proto"]"#),
        "should export echo.proto"
    );
    assert!(
        actr.contains(r#"name = "EchoService""#),
        "actr_type.name should be EchoService"
    );
    assert!(
        !actr.contains("echo-echo-server"),
        "service should have no remote deps"
    );

    // -- main.rs --
    let main = std::fs::read_to_string(dir.join("src/main.rs")).unwrap();
    assert!(
        !main.contains("discover_route_candidates"),
        "service should not discover"
    );
    assert!(
        main.contains("package-backed Actor-RTC EchoService host"),
        "service main should describe the package-backed host flow"
    );
    assert!(
        main.contains("Source-defined Rust service workloads were removed"),
        "service main should explain that source-defined workloads were removed"
    );

    // -- echo_service.rs --
    let svc = std::fs::read_to_string(dir.join("src/echo_service.rs")).unwrap();
    assert!(
        svc.contains("echo_actor::EchoServiceHandler"),
        "import EchoServiceHandler"
    );
    assert!(
        svc.contains("echo::"),
        "import message types from echo module"
    );
    assert!(
        svc.contains("_ctx: &C") || svc.contains("ctx: &C"),
        "handler ctx param"
    );

    // -- local echo.proto --
    let proto = std::fs::read_to_string(dir.join("protos/local/echo.proto")).unwrap();
    assert!(
        proto.contains("service EchoService"),
        "should define EchoService"
    );
    assert!(proto.contains("rpc Echo"), "should declare Echo rpc");
}

#[test]
fn rust_echo_both_app_uses_local_service_dependency() {
    let tmp = TempDir::new().unwrap();
    let dir = init_rust_echo_both(tmp.path(), "echo-pair");

    let app_actr = std::fs::read_to_string(dir.join("echo-app/actr.toml")).unwrap();
    assert!(
        app_actr.contains("EchoService = {}"),
        "role=both app should depend on local echo-service, got:\n{app_actr}"
    );
    assert!(
        !app_actr.contains("echo-echo-server"),
        "role=both app should not depend on remote echo-echo-server"
    );
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[test]
fn rust_echo_init_fails_if_directory_exists() {
    let tmp = TempDir::new().unwrap();
    init_rust_echo_app(tmp.path(), "duplicate-svc");

    let out = run_actr(
        &[
            "init",
            "-l",
            "rust",
            "--template",
            "echo",
            "--role",
            "app",
            "--signaling",
            "wss://actrix1.develenv.com",
            "duplicate-svc",
        ],
        tmp.path(),
    );
    assert!(!out.status.success(), "second init should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("already exists") || stderr.contains("exist"),
        "should mention existing directory, got:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Rust codegen: bridge local_actor generation
// ---------------------------------------------------------------------------

const ECHO_PROTO: &str = r#"syntax = "proto3";
package echo;
message EchoRequest { string message = 1; }
message EchoResponse { string reply = 1; uint64 timestamp = 2; }
service EchoService {
  rpc Echo(EchoRequest) returns (EchoResponse);
}
"#;

const CHAT_PROTO: &str = r#"syntax = "proto3";
package chat;
message SendMessageRequest { string room_id = 1; string content = 2; }
message SendMessageResponse { string message_id = 1; }
message JoinRoomRequest { string room_id = 1; string user_id = 2; }
message JoinRoomResponse { bool success = 1; }
service ChatService {
  rpc SendMessage(SendMessageRequest) returns (SendMessageResponse);
  rpc JoinRoom(JoinRoomRequest) returns (JoinRoomResponse);
}
"#;

/// Helper: write a minimal Actr.lock.toml so `actr gen` doesn't abort.
fn write_lock_toml(dir: &std::path::Path) {
    std::fs::write(
        dir.join("Actr.lock.toml"),
        "[metadata]\nversion = 1\ngenerated_at = \"2026-01-01T00:00:00Z\"\n",
    )
    .unwrap();
}

/// Helper: write a remote proto under protos/remote/<dep-name>/<stem>.proto
fn write_remote_proto(dir: &std::path::Path, dep_name: &str, stem: &str, content: &str) {
    let proto_dir = dir.join("protos/remote").join(dep_name);
    std::fs::create_dir_all(&proto_dir).unwrap();
    std::fs::write(proto_dir.join(format!("{stem}.proto")), content).unwrap();
}

/// Verify that `actr gen` generates local_actor.rs for the bridge service and skips local_service.rs.
#[test]
fn rust_echo_app_gen_single_remote_service() {
    let tmp = TempDir::new().unwrap();
    let dir = init_rust_echo_app(tmp.path(), "single-svc-app");

    write_lock_toml(&dir);
    write_remote_proto(&dir, "echo-echo-server", "echo", ECHO_PROTO);

    let out = run_actr(&["gen", "-l", "rust"], &dir);
    assert_actr_success(&out, "actr gen (single remote service)");

    let local_actor = std::fs::read_to_string(dir.join("src/generated/local_actor.rs")).unwrap();

    assert!(
        local_actor.contains("pub trait SingleSvcAppClientAppHandler"),
        "should generate bridge handler trait"
    );
    assert!(
        local_actor.contains("pub struct SingleSvcAppClientAppWorkload"),
        "should generate bridge workload"
    );
    assert!(
        local_actor.contains("\"echo.EchoService.Echo\""),
        "should forward the Echo route key"
    );
    assert!(
        local_actor.contains("manufacturer: \"acme\".to_string()")
            && local_actor.contains("name: \"EchoService\".to_string()"),
        "should use the mapped remote actr_type"
    );
    assert!(
        !dir.join("src/local_service.rs").exists(),
        "empty bridge proto should not generate local_service.rs"
    );
}

/// Verify that `actr gen` merges multiple remote services into local_actor.rs.
#[test]
fn rust_echo_app_gen_two_remote_services() {
    let tmp = TempDir::new().unwrap();
    let dir = init_rust_echo_app(tmp.path(), "two-svc-app");

    write_lock_toml(&dir);
    write_remote_proto(&dir, "echo-echo-server", "echo", ECHO_PROTO);
    write_remote_proto(&dir, "chat-service", "chat", CHAT_PROTO);

    let out = run_actr(&["gen", "-l", "rust"], &dir);
    assert_actr_success(&out, "actr gen (two remote services)");

    let local_actor = std::fs::read_to_string(dir.join("src/generated/local_actor.rs")).unwrap();

    assert!(
        local_actor.contains("\"echo.EchoService.Echo\""),
        "should forward EchoService"
    );
    assert!(
        local_actor.contains("\"chat.ChatService.SendMessage\""),
        "should forward ChatService.SendMessage"
    );
    assert!(
        local_actor.contains("\"chat.ChatService.JoinRoom\""),
        "should forward ChatService.JoinRoom"
    );
    assert!(
        local_actor.contains("manufacturer: \"acme\".to_string()")
            && local_actor.contains("name: \"EchoService\".to_string()"),
        "should include EchoService actr_type"
    );
    assert!(
        local_actor.contains("manufacturer: \"acme\".to_string()")
            && local_actor.contains("name: \"ChatService\".to_string()"),
        "should include ChatService actr_type"
    );
    assert!(
        !dir.join("src/local_service.rs").exists(),
        "empty bridge proto should still skip local_service.rs"
    );
}
