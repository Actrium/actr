//! End-to-end tests for Swift echo template with public registry
//!
//! These tests verify the full workflow: init → install → gen → inject CLI target → build → run.
//! They require Xcode, xcodegen, and network access.
//!
//! Run with: `cargo test --test e2e_swift_echo -- --ignored --test-threads=1`

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

/// Inject a macOS CLI target into the xcodegen project.yml so we can build
/// and run a command-line binary that exercises the echo RPC.
fn inject_cli_target(project_dir: &std::path::Path, project_name: &str) {
    // 1. Append EchoCLI target to project.yml
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

    // 2. Create EchoCLI/main.swift
    let cli_dir = project_dir.join("EchoCLI");
    std::fs::create_dir_all(&cli_dir).expect("create EchoCLI dir");

    // Use the generated EchoAppWorkload (from local.actor.swift) and add Workload conformance.
    // The generated workload's __dispatch handles routing echo RPCs to the remote service.
    let main_swift = r#"import Actr
import Foundation
import SwiftProtobuf

// Workload conformance for the generated EchoAppWorkload
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

/// End-to-end: Swift echo app with public registry.
/// init → install → gen → inject CLI target → xcodegen → xcodebuild → run
#[test]
#[ignore] // Requires Xcode, xcodegen, and network access
fn swift_echo_e2e_app_with_public_registry() {
    let tmp = TempDir::new().unwrap();
    let project_name = "EchoApp";

    // 1. actr init
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
            "wss://actrix1.develenv.com",
            project_name,
        ],
        tmp.path(),
    );
    assert_actr_success(&out, "actr init");

    let project_dir = tmp.path().join(project_name);
    assert!(project_dir.exists(), "project dir should exist");

    // 2. Verify Actr.toml references acme+EchoService
    let actr_toml = std::fs::read_to_string(project_dir.join("Actr.toml")).unwrap();
    assert!(
        actr_toml.contains("acme+EchoService"),
        "Actr.toml should reference acme+EchoService, got:\n{actr_toml}"
    );

    // 3. actr install
    let out = run_actr(&["install"], &project_dir);
    assert_actr_success(&out, "actr install");

    // 4. Verify remote proto downloaded
    assert!(
        project_dir
            .join("protos/remote/echo-echo-server/echo.proto")
            .exists(),
        "echo.proto should be downloaded"
    );

    // 5. actr gen -l swift
    let out = run_actr(&["gen", "-l", "swift"], &project_dir);
    assert_actr_success(&out, "actr gen");

    // 6. Verify generated files
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

    // 7. Inject macOS CLI target
    inject_cli_target(&project_dir, project_name);

    // 8. xcodegen generate
    let out = Command::new("xcodegen")
        .args(["generate"])
        .current_dir(&project_dir)
        .output()
        .expect("xcodegen not found");
    assert!(
        out.status.success(),
        "xcodegen generate failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // 9. xcodebuild build EchoCLI
    let out = Command::new("xcodebuild")
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
    assert!(
        out.status.success(),
        "xcodebuild build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // 10. Run the CLI binary
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

    println!("Swift echo e2e with public registry passed");
}
