//! End-to-end tests for Swift echo template.
//!
//! These tests run against local Actrix and local Rust echo service only.
//! Run with: `cargo test --test e2e_swift_echo -- --ignored --test-threads=1`

mod e2e_support;

use std::process::Command;

use e2e_support::{
    LocalActrix, LocalRustEchoService, align_project_with_local_actrix, assert_success,
    ensure_local_swift_xcframework, run_actr,
};
use tempfile::TempDir;

/// Inject a macOS CLI target into project.yml so we can run a command-line binary.
fn inject_cli_target(project_dir: &std::path::Path, project_name: &str) {
    let project_yml = project_dir.join("project.yml");
    let mut yml = std::fs::read_to_string(&project_yml).expect("read project.yml");

    let cli_target = format!(
        r#"
  EchoCLI:
    type: tool
    platform: macOS
    deploymentTarget: "13.0"
    sources:
      - path: EchoCLI
      - path: {project_name}/Generated
    settings:
      SWIFT_VERSION: "6.0"
    dependencies:
      - package: actr-swift
        product: Actr
      - package: actr-protocols
        product: ActrProtocols
      - package: swift-protobuf
        product: SwiftProtobuf
"#
    );
    yml.push_str(&cli_target);
    std::fs::write(&project_yml, yml).expect("write project.yml");

    let cli_dir = project_dir.join("EchoCLI");
    std::fs::create_dir_all(&cli_dir).expect("create EchoCLI dir");

    let main_swift = r#"import Actr
import Foundation
import SwiftProtobuf

extension EchoAppWorkload: Workload {
    public func onStart(ctx _: Context) async throws {}
    public func onStop(ctx _: Context) async throws {}

    public func dispatch(ctx: Context, envelope: RpcEnvelope) async throws -> Data {
        return try await __dispatch(ctx: ctx, envelope: envelope)
    }
}

@main
struct EchoCLI {
    static func main() async throws {
        let cwd = FileManager.default.currentDirectoryPath
        let configPath = (cwd as NSString).appendingPathComponent("Actr.toml")

        guard FileManager.default.fileExists(atPath: configPath) else {
            fputs("Error: Actr.toml not found at \(configPath)\n", stderr)
            exit(1)
        }

        let system = try await ActrSystem.from(tomlConfig: configPath)
        let workload = EchoAppWorkload()
        let node = try system.spawn(workload: workload)
        let actr = try await node.start()

        var request = Echo_EchoRequest()
        request.message = "hello"

        let response: Echo_EchoResponse = try await actr.call(request)
        print("Echo reply: \(response.reply)")

        await actr.stop()
    }
}
"#;
    std::fs::write(cli_dir.join("EchoCLI.swift"), main_swift).expect("write EchoCLI.swift");
}

#[test]
#[ignore] // Requires Xcode and xcodegen
fn swift_echo_e2e_app_with_local_registry() {
    let actrix = LocalActrix::start().expect("failed to start local actrix");
    let registry = LocalRustEchoService::start(&actrix.signaling_ws_url)
        .expect("failed to start local rust echo service");
    let tmp = TempDir::new().unwrap();
    let project_name = "EchoApp";

    let out = run_actr(
        &[
            "init",
            "-l",
            "swift",
            "--template",
            "echo",
            "--role",
            "app",
            "--signaling",
            &actrix.signaling_ws_url,
            project_name,
        ],
        tmp.path(),
    );
    assert_success(&out, "actr init");

    let project_dir = tmp.path().join(project_name);
    assert!(project_dir.exists(), "project dir should exist");
    align_project_with_local_actrix(&project_dir).expect("failed to set local realm for app");

    let actr_toml = std::fs::read_to_string(project_dir.join("Actr.toml")).unwrap();
    assert!(
        actr_toml.contains("acme:EchoService"),
        "Actr.toml should reference acme:EchoService, got:\n{actr_toml}"
    );

    let out = run_actr(&["install"], &project_dir);
    assert_success(&out, "actr install");
    assert!(
        project_dir
            .join("protos/remote/echo-echo-server/echo.proto")
            .exists(),
        "echo.proto should be downloaded"
    );

    let out = run_actr(&["gen", "-l", "swift"], &project_dir);
    assert_success(&out, "actr gen");

    let gen_dir = project_dir.join(project_name).join("Generated");
    assert!(gen_dir.join("echo.pb.swift").exists(), "echo.pb.swift");
    assert!(
        gen_dir.join("echo.client.swift").exists(),
        "echo.client.swift"
    );
    assert!(
        gen_dir.join("local.actor.swift").exists(),
        "local.actor.swift"
    );

    inject_cli_target(&project_dir, project_name);

    let local_xcframework =
        ensure_local_swift_xcframework().expect("failed to prepare local swift xcframework");

    let out = Command::new("xcodegen")
        .args(["generate"])
        .current_dir(&project_dir)
        .output()
        .expect("xcodegen not found");
    assert_success(&out, "xcodegen generate");

    let out = Command::new("xcodebuild")
        .env("ACTR_BINARY_PATH", &local_xcframework)
        .args([
            "build",
            "-project",
            &format!("{project_name}.xcodeproj"),
            "-scheme",
            "EchoCLI",
            "-configuration",
            "Debug",
            "-derivedDataPath",
            "build",
            "ONLY_ACTIVE_ARCH=YES",
        ])
        .current_dir(&project_dir)
        .output()
        .expect("xcodebuild not found");
    assert_success(&out, "xcodebuild build");

    let cli_binary = project_dir.join("build/Build/Products/Debug/EchoCLI");
    assert!(
        cli_binary.exists(),
        "EchoCLI binary should exist at {}",
        cli_binary.display()
    );

    let out = Command::new(&cli_binary)
        .current_dir(&project_dir)
        .output()
        .expect("failed to run EchoCLI");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "EchoCLI failed:\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("Echo reply:"),
        "Expected 'Echo reply:' in stdout, got:\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        registry.logs().contains("Received echo request: hello"),
        "local registry rust service did not receive request:\n{}",
        registry.logs()
    );
}
