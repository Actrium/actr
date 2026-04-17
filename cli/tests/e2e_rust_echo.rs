//! End-to-end tests for Rust echo template.
//!
//! These tests run against a mock signaling server (no real actrix needed).
//! Run with: `cargo test --test e2e_rust_echo -- --ignored --test-threads=1`

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use actr_cli::test_support::{
    LoggedProcess, MockSignaling, align_project_with_local_actrix,
    align_rust_project_with_workspace, assert_success, cargo_build,
    pin_echo_service_dependency_version, random_manufacturer, run_actr, rust_e2e_target_dir,
};
use tempfile::TempDir;

#[test]
#[ignore] // Slow e2e (~200s), run explicitly with --ignored
fn rust_echo_e2e_service_and_app() {
    let signaling = MockSignaling::start().expect("failed to start mock signaling");
    let tmp = TempDir::new().unwrap();
    let mfr = random_manufacturer();

    let init_out = run_actr(
        &[
            "init",
            "-l",
            "rust",
            "--template",
            "echo",
            "--role",
            "both",
            "--signaling",
            &signaling.signaling_ws_url,
            "--manufacturer",
            &mfr,
            "e2e",
        ],
        tmp.path(),
    );
    assert_success(&init_out, "actr init --role both");

    let svc_dir = tmp.path().join("e2e/echo-service");
    let app_dir = tmp.path().join("e2e/echo-app");
    assert!(svc_dir.exists(), "echo-service dir");
    assert!(app_dir.exists(), "echo-app dir");
    align_project_with_local_actrix(&svc_dir).expect("failed to set local realm for svc");
    align_project_with_local_actrix(&app_dir).expect("failed to set local realm for app");
    pin_echo_service_dependency_version(&app_dir, &mfr)
        .expect("failed to pin app echo dependency version");
    align_rust_project_with_workspace(&svc_dir).expect("failed to patch svc Cargo.toml");
    align_rust_project_with_workspace(&app_dir).expect("failed to patch app Cargo.toml");

    assert_success(
        &run_actr(&["deps", "install"], &svc_dir),
        "actr deps install (svc)",
    );
    assert_success(
        &run_actr(&["gen", "-l", "rust"], &svc_dir),
        "actr gen (svc)",
    );
    cargo_build(&svc_dir);

    let mut svc_cmd = Command::new("cargo");
    svc_cmd
        .args(["run"])
        .current_dir(&svc_dir)
        .env("CARGO_TARGET_DIR", rust_e2e_target_dir());
    let mut svc = LoggedProcess::spawn(svc_cmd, "rust-e2e-service").expect("start rust service");
    assert!(
        svc.wait_for_log("EchoService registered", Duration::from_secs(180)),
        "service not ready within timeout:\n{}",
        svc.logs()
    );

    assert_success(
        &run_actr(&["deps", "install"], &app_dir),
        "actr deps install (app)",
    );
    assert_success(
        &run_actr(&["gen", "-l", "rust"], &app_dir),
        "actr gen (app)",
    );
    cargo_build(&app_dir);

    let mut app = Command::new("cargo")
        .args(["run"])
        .current_dir(&app_dir)
        .env("CARGO_TARGET_DIR", rust_e2e_target_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match app.try_wait().unwrap() {
            Some(_) => break,
            None if Instant::now() > deadline => {
                app.kill().ok();
                panic!("app did not exit within 60s");
            }
            None => std::thread::sleep(Duration::from_millis(500)),
        }
    }

    let app_out = app.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&app_out.stdout);
    let stderr = String::from_utf8_lossy(&app_out.stderr);
    assert!(
        app_out.status.success(),
        "app failed:\nstdout: {stdout}\nstderr: {stderr}\nservice logs:\n{}",
        svc.logs(),
    );
    assert!(
        stdout.contains("Echo reply:"),
        "missing echo reply in:\n{stdout}"
    );
    assert!(
        svc.logs().contains("Received echo request: hello"),
        "service missing request log:\n{}",
        svc.logs()
    );
}
