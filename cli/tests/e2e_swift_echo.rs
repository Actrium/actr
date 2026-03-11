//! End-to-end tests for Swift echo template.
//!
//! These tests run against a local Actrix instance and local Swift projects only.
//! Run with: `cargo test --test e2e_swift_echo -- --ignored --test-threads=1`

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use actr_cli::test_support::{
    LocalActrix, LoggedProcess, align_project_with_local_actrix, assert_success,
    ensure_local_swift_xcframework, pin_echo_service_dependency_version, run_actr,
};
use tempfile::TempDir;

fn append_cli_target(project_dir: &std::path::Path, project_name: &str, target_name: &str) {
    let project_yml = project_dir.join("project.yml");
    let mut yml = std::fs::read_to_string(&project_yml).expect("read project.yml");
    let cli_target = format!(
        r#"
  {target_name}:
    type: tool
    platform: macOS
    deploymentTarget: "13.0"
    sources:
      - path: {target_name}
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
    std::fs::create_dir_all(project_dir.join(target_name)).expect("create CLI target dir");
}

fn write_service_cli(project_dir: &std::path::Path) {
    let source = r#"import Actr
import Foundation
import SwiftProtobuf

public final class EchoServiceHandlerImpl: EchoServiceHandler {
    public init() {}

    public func echo(req: Echo_EchoRequest, ctx _: Context) async throws -> Echo_EchoResponse {
        print("Received echo request: \(req.message)")
        var response = Echo_EchoResponse()
        response.reply = req.message
        return response
    }
}

extension EchoServiceWorkload: Workload where T == EchoServiceHandlerImpl {
    public func onStart(ctx _: Context) async throws {}
    public func onStop(ctx _: Context) async throws {}

    public func dispatch(ctx: Context, envelope: RpcEnvelope) async throws -> Data {
        return try await __dispatch(ctx: ctx, envelope: envelope)
    }
}

@main
struct EchoServiceCLI {
    static func main() async throws {
        let cwd = FileManager.default.currentDirectoryPath
        let configPath = (cwd as NSString).appendingPathComponent("actr.toml")

        let system = try await ActrSystem.from(tomlConfig: configPath)
        let workload = EchoServiceWorkload(handler: EchoServiceHandlerImpl())
        let node = try system.spawn(workload: workload)
        let _ = try await node.start()
        print("EchoService registered")

        while true {
            try await Task.sleep(nanoseconds: 1_000_000_000)
        }
    }
}
"#;
    std::fs::write(
        project_dir.join("EchoServiceCLI/EchoServiceCLI.swift"),
        source,
    )
    .expect("write EchoServiceCLI.swift");
}

fn write_app_cli(project_dir: &std::path::Path) {
    let source = r#"import Actr
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
struct EchoAppCLI {
    static func main() async throws {
        let cwd = FileManager.default.currentDirectoryPath
        let configPath = (cwd as NSString).appendingPathComponent("actr.toml")

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
    std::fs::write(project_dir.join("EchoAppCLI/EchoAppCLI.swift"), source)
        .expect("write EchoAppCLI.swift");
}

fn generate_xcode_project(project_dir: &std::path::Path) {
    let out = Command::new("xcodegen")
        .args(["generate"])
        .current_dir(project_dir)
        .output()
        .expect("xcodegen not found");
    assert_success(&out, "xcodegen generate");
}

fn build_cli_binary(
    project_dir: &std::path::Path,
    project_name: &str,
    scheme: &str,
    local_xcframework: &std::path::Path,
) -> std::path::PathBuf {
    let out = Command::new("xcodebuild")
        .env("ACTR_BINARY_PATH", local_xcframework)
        .args([
            "build",
            "-project",
            &format!("{project_name}.xcodeproj"),
            "-scheme",
            scheme,
            "-configuration",
            "Debug",
            "-derivedDataPath",
            "build",
            "ONLY_ACTIVE_ARCH=YES",
            "CODE_SIGNING_ALLOWED=NO",
        ])
        .current_dir(project_dir)
        .output()
        .expect("xcodebuild not found");
    assert_success(&out, &format!("xcodebuild build ({scheme})"));

    let binary = project_dir.join(format!("build/Build/Products/Debug/{scheme}"));
    assert!(
        binary.exists(),
        "{scheme} binary should exist at {}",
        binary.display()
    );
    binary
}

#[test]
#[ignore] // Requires macOS, Xcode, xcodegen, and local Swift bindings
fn swift_echo_e2e_service_and_app() {
    let local_xcframework =
        ensure_local_swift_xcframework().expect("failed to prepare local swift xcframework");
    let actrix = LocalActrix::start().expect("failed to start local actrix");
    let tmp = TempDir::new().unwrap();

    let init_out = run_actr(
        &[
            "init",
            "-l",
            "swift",
            "--template",
            "echo",
            "--role",
            "both",
            "--signaling",
            &actrix.signaling_ws_url,
            "--manufacturer",
            "swift-e2e",
            "e2e-swift",
        ],
        tmp.path(),
    );
    assert_success(&init_out, "actr init -l swift --role both");

    let svc_dir = tmp.path().join("e2e-swift/echo-service");
    let app_dir = tmp.path().join("e2e-swift/echo-app");
    assert!(svc_dir.exists(), "echo-service dir should exist");
    assert!(app_dir.exists(), "echo-app dir should exist");
    align_project_with_local_actrix(&svc_dir).expect("failed to set local realm for service");
    align_project_with_local_actrix(&app_dir).expect("failed to set local realm for app");
    pin_echo_service_dependency_version(&app_dir, "swift-e2e")
        .expect("failed to pin app echo dependency version");

    assert_success(&run_actr(&["install"], &svc_dir), "actr install (svc)");
    assert_success(
        &run_actr(&["gen", "-l", "swift"], &svc_dir),
        "actr gen -l swift (svc)",
    );
    append_cli_target(&svc_dir, "EchoService", "EchoServiceCLI");
    write_service_cli(&svc_dir);
    generate_xcode_project(&svc_dir);
    let svc_binary = build_cli_binary(
        &svc_dir,
        "EchoService",
        "EchoServiceCLI",
        &local_xcframework,
    );

    let mut svc_cmd = Command::new(&svc_binary);
    svc_cmd
        .current_dir(&svc_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut svc =
        LoggedProcess::spawn(svc_cmd, "swift-e2e-service").expect("failed to start service");
    assert!(
        svc.wait_for_log("EchoService registered", Duration::from_secs(180)),
        "service not ready within timeout:\n{}",
        svc.logs()
    );

    assert_success(&run_actr(&["install"], &app_dir), "actr install (app)");
    assert_success(
        &run_actr(&["gen", "-l", "swift"], &app_dir),
        "actr gen -l swift (app)",
    );
    append_cli_target(&app_dir, "EchoApp", "EchoAppCLI");
    write_app_cli(&app_dir);
    generate_xcode_project(&app_dir);
    let app_binary = build_cli_binary(&app_dir, "EchoApp", "EchoAppCLI", &local_xcframework);

    let mut app = Command::new(&app_binary)
        .current_dir(&app_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start app");

    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match app.try_wait().unwrap() {
            Some(_) => break,
            None if Instant::now() > deadline => {
                app.kill().ok();
                let app_out = app.wait_with_output().unwrap();
                panic!(
                    "app did not exit within 120s:\nstdout: {}\nstderr: {}\nservice:\n{}",
                    String::from_utf8_lossy(&app_out.stdout),
                    String::from_utf8_lossy(&app_out.stderr),
                    svc.logs()
                );
            }
            None => std::thread::sleep(Duration::from_millis(500)),
        }
    }

    let app_out = app.wait_with_output().unwrap();
    let app_stdout = String::from_utf8_lossy(&app_out.stdout);
    let app_stderr = String::from_utf8_lossy(&app_out.stderr);
    assert!(
        app_out.status.success(),
        "app failed:\nstdout: {app_stdout}\nstderr: {app_stderr}\nservice logs:\n{}\nactrix logs:\n{}",
        svc.logs(),
        actrix.logs()
    );
    assert!(
        app_stdout.contains("Echo reply: hello"),
        "missing echo reply in app output:\nstdout: {app_stdout}\nstderr: {app_stderr}"
    );
    assert!(
        svc.logs().contains("Received echo request: hello"),
        "service missing request log:\n{}",
        svc.logs()
    );
}
