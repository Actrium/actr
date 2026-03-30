//! End-to-end tests for TypeScript echo template.
//!
//! These tests run against local Actrix and local echo services only.
//! Run with: `cargo test --test e2e_typescript_echo -- --ignored --test-threads=1`

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use actr_cli::test_support as e2e_support;
use actr_cli::test_support::{
    LocalActrix, LoggedProcess, align_project_with_local_actrix, assert_success,
    pin_echo_service_dependency_version, random_manufacturer,
};
use tempfile::TempDir;

const EXPECTED_ACTR_TS_VERSION: &str = "0.1.14";

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
        assert_success(
            &run_npm(&["install"], &dir),
            "npm install (tools/protoc-gen/typescript)",
        );
        assert_success(
            &run_npm(&["run", "bundle"], &dir),
            "npm run bundle (tools/protoc-gen/typescript)",
        );
        dir
    })
}

fn run_actr(args: &[&str], cwd: &Path) -> Output {
    let tool_dir = prepare_typescript_codegen_tools();
    let mut path_entries = vec![tool_dir.join("scripts"), tool_dir.join("node_modules/.bin")];
    if let Some(existing) = std::env::var_os("PATH") {
        path_entries.extend(std::env::split_paths(&existing));
    }
    let path = std::env::join_paths(path_entries).expect("failed to construct PATH");

    Command::new(e2e_support::actr_bin())
        .args(args)
        .current_dir(cwd)
        .env("PATH", path)
        .output()
        .expect("failed to run actr binary")
}

fn run_npm(args: &[&str], cwd: &Path) -> Output {
    Command::new("npm")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run npm")
}

fn run_npm_with_path(args: &[&str], cwd: &Path, path: &OsString) -> Output {
    Command::new("npm")
        .args(args)
        .current_dir(cwd)
        .env("PATH", path)
        .output()
        .expect("failed to run npm")
}

fn e2e_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn assert_generated_actr_dependency(project_dir: &Path) {
    let package_json_path = project_dir.join("package.json");
    let raw =
        std::fs::read_to_string(&package_json_path).expect("failed to read generated package.json");
    let value: serde_json::Value =
        serde_json::from_str(&raw).expect("failed to parse generated package.json");

    let actual = value
        .get("dependencies")
        .and_then(serde_json::Value::as_object)
        .and_then(|deps| deps.get("@actrium/actr"))
        .and_then(serde_json::Value::as_str)
        .expect("@actrium/actr dependency missing in package.json");

    assert_eq!(
        actual, EXPECTED_ACTR_TS_VERSION,
        "generated package.json should use the published npm package version"
    );
}

fn dev_path_env() -> OsString {
    let tool_dir = prepare_typescript_codegen_tools();
    let mut path_entries = vec![tool_dir.join("scripts"), tool_dir.join("node_modules/.bin")];
    if let Some(existing) = std::env::var_os("PATH") {
        path_entries.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(path_entries).expect("failed to construct PATH")
}

#[test]
#[ignore] // Slow e2e, run explicitly in dedicated CI
fn typescript_echo_e2e_service_and_app() {
    let _guard = e2e_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let actrix = LocalActrix::start().expect("failed to start local actrix");
    let tmp = TempDir::new().unwrap();
    let mfr = random_manufacturer();

    let out = run_actr(
        &[
            "init",
            "-l",
            "typescript",
            "--template",
            "echo",
            "--role",
            "both",
            "--signaling",
            &actrix.signaling_ws_url,
            "--manufacturer",
            &mfr,
            "e2e-ts",
        ],
        tmp.path(),
    );
    assert_success(&out, "actr init -l typescript --role both");

    let svc_dir = tmp.path().join("e2e-ts/echo-service");
    let app_dir = tmp.path().join("e2e-ts/echo-app");
    assert!(svc_dir.exists(), "echo-service dir should exist");
    assert!(app_dir.exists(), "echo-app dir should exist");
    align_project_with_local_actrix(&svc_dir).expect("failed to set local realm for service");
    align_project_with_local_actrix(&app_dir).expect("failed to set local realm for app");
    pin_echo_service_dependency_version(&app_dir, &mfr)
        .expect("failed to pin app echo dependency version");
    assert_generated_actr_dependency(&svc_dir);
    assert_generated_actr_dependency(&app_dir);

    assert_success(&run_actr(&["install"], &svc_dir), "actr install (svc)");
    assert_success(
        &run_actr(&["gen", "-l", "typescript"], &svc_dir),
        "actr gen -l typescript (svc)",
    );

    let path = dev_path_env();
    let mut svc_cmd = Command::new("npm");
    svc_cmd
        .args(["run", "dev"])
        .current_dir(&svc_dir)
        .env("PATH", &path);
    let mut svc = LoggedProcess::spawn(svc_cmd, "ts-e2e-service").expect("failed to start service");
    assert!(
        svc.wait_for_log("EchoService registered", Duration::from_secs(180)),
        "service not ready within timeout:\n{}",
        svc.logs()
    );

    assert_success(&run_actr(&["install"], &app_dir), "actr install (app)");
    assert_success(
        &run_actr(&["gen", "-l", "typescript"], &app_dir),
        "actr gen -l typescript (app)",
    );
    assert_success(
        &run_npm_with_path(&["run", "typecheck"], &app_dir, &path),
        "npm run typecheck (app)",
    );
    std::fs::remove_file(app_dir.join("manifest.lock.toml"))
        .expect("failed to remove app lock file");

    let mut app = Command::new("npm")
        .args(["run", "dev"])
        .current_dir(&app_dir)
        .env("PATH", &path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start app");

    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        match app.try_wait().unwrap() {
            Some(_) => break,
            None if Instant::now() > deadline => {
                app.kill().ok();
                let app_out = app.wait_with_output().unwrap();
                panic!(
                    "app did not exit within 90s:\nstdout: {}\nstderr: {}\nservice:\n{}",
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
        "app failed:\nstdout: {app_stdout}\nstderr: {app_stderr}"
    );
    assert!(
        app_stdout.contains("Echo reply:"),
        "missing echo reply in app output:\nstdout: {app_stdout}\nstderr: {app_stderr}"
    );
    assert!(
        svc.logs().contains("Received echo request: hello"),
        "service missing request log:\n{}",
        svc.logs()
    );
}
