//! End-to-end tests for TypeScript echo template.
//!
//! These tests build and run real service + app communication over the signaling server.
//! They require git, network access, and Node.js.
//!
//! Run with:
//! `cargo test --test e2e_typescript_echo -- --test-threads=1`

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

fn actr_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_actr"))
}

const EXPECTED_ACTR_TS_TARBALL_URL: &str =
    "https://github.com/actor-rtc/actr-ts/releases/download/v0.1.14/actor-rtc-actr-0.1.14.tgz";

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

    Command::new(actr_bin())
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

fn assert_success(out: &Output, context: &str) {
    assert!(
        out.status.success(),
        "{context} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn random_manufacturer() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("test{nanos:08x}")
}

/// Wait until child output contains `needle` or timeout expires.
fn wait_for_output(
    child: &mut std::process::Child,
    needle: &str,
    timeout: std::time::Duration,
) -> (bool, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::BufRead;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    let found = Arc::new(AtomicBool::new(false));
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));

    macro_rules! drain {
        ($stream:expr) => {
            if let Some(s) = $stream {
                let f = Arc::clone(&found);
                let l = Arc::clone(&lines);
                let n = needle.to_string();
                std::thread::spawn(move || {
                    for line in std::io::BufReader::new(s).lines().flatten() {
                        if line.contains(&n) {
                            f.store(true, Ordering::SeqCst);
                        }
                        l.lock().unwrap().push(line);
                    }
                });
            }
        };
    }

    drain!(child.stdout.take());
    drain!(child.stderr.take());

    let deadline = std::time::Instant::now() + timeout;
    while !found.load(std::sync::atomic::Ordering::SeqCst) {
        if std::time::Instant::now() > deadline {
            return (false, lines);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    (true, lines)
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
        .and_then(|deps| deps.get("@actor-rtc/actr"))
        .and_then(serde_json::Value::as_str)
        .expect("@actor-rtc/actr dependency missing in package.json");

    assert_eq!(
        actual, EXPECTED_ACTR_TS_TARBALL_URL,
        "generated package.json should use the release tarball dependency"
    );
}

#[test]
fn typescript_echo_e2e_service_and_app() {
    let _guard = e2e_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let tmp = TempDir::new().unwrap();
    let mfr = random_manufacturer();

    // 1. init two projects via role=both
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
            "wss://actrix1.develenv.com",
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
    assert_generated_actr_dependency(&svc_dir);
    assert_generated_actr_dependency(&app_dir);

    // 2. Service workflow: actr install -> actr gen
    assert_success(&run_actr(&["install"], &svc_dir), "actr install (svc)");
    assert_success(
        &run_actr(&["gen", "-l", "typescript"], &svc_dir),
        "actr gen -l typescript (svc)",
    );

    // 3. Start service and wait for registration log.
    let mut svc = Command::new("npm")
        .args(["run", "dev"])
        .current_dir(&svc_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start service");

    let (ready, svc_log) = wait_for_output(
        &mut svc,
        "EchoService registered",
        std::time::Duration::from_secs(120),
    );
    if !ready {
        let svc_output = svc_log.lock().unwrap().join("\n");
        svc.kill().ok();
        svc.wait().ok();
        panic!("service not ready within 120s (manufacturer={mfr}):\n{svc_output}");
    }

    // 4. App workflow: actr install -> actr gen -> typecheck
    assert_success(&run_actr(&["install"], &app_dir), "actr install (app)");
    assert_success(
        &run_actr(&["gen", "-l", "typescript"], &app_dir),
        "actr gen -l typescript (app)",
    );
    assert_success(
        &run_npm(&["run", "typecheck"], &app_dir),
        "npm run typecheck (app)",
    );

    // 5. Run app against the live service and wait for it to exit.
    let mut app = Command::new("npm")
        .args(["run", "dev"])
        .current_dir(&app_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start app");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    loop {
        match app.try_wait().unwrap() {
            Some(_) => break,
            None if std::time::Instant::now() > deadline => {
                app.kill().ok();
                let app_out = app.wait_with_output().unwrap();
                let app_stdout = String::from_utf8_lossy(&app_out.stdout);
                let app_stderr = String::from_utf8_lossy(&app_out.stderr);
                let svc_output = svc_log.lock().unwrap().join("\n");
                svc.kill().ok();
                svc.wait().ok();
                panic!(
                    "app did not exit within 90s:\nstdout: {app_stdout}\nstderr: {app_stderr}\nservice:\n{svc_output}"
                );
            }
            None => std::thread::sleep(std::time::Duration::from_millis(500)),
        }
    }

    let app_out = app.wait_with_output().unwrap();

    // 6. Confirm the round-trip completed and service remained healthy, then clean up.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let app_stdout = String::from_utf8_lossy(&app_out.stdout);
    let app_stderr = String::from_utf8_lossy(&app_out.stderr);
    let svc_output = svc_log.lock().unwrap().join("\n");
    svc.kill().ok();
    svc.wait().ok();

    assert!(
        app_out.status.success(),
        "app failed:\nstdout: {app_stdout}\nstderr: {app_stderr}"
    );
    assert!(
        app_stdout.contains("Echo reply:"),
        "missing echo reply in app output:\nstdout: {app_stdout}\nstderr: {app_stderr}"
    );
    assert!(
        svc_output.contains("Received echo request: hello"),
        "service missing request log in:\n{svc_output}"
    );
    assert!(
        svc_output.contains("EchoService registered"),
        "missing register log in service output:\n{svc_output}"
    );
    assert!(
        svc_output.contains("EchoService workload started"),
        "service missing startup log in:\n{svc_output}"
    );
}

#[test]
fn typescript_echo_e2e_app_with_public_registry() {
    let _guard = e2e_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let tmp = TempDir::new().unwrap();

    // 1. actr init --role app (uses default manufacturer 'acme')
    let out = run_actr(
        &[
            "init",
            "-l",
            "typescript",
            "--template",
            "echo",
            "--role",
            "app",
            "--signaling",
            "wss://actrix1.develenv.com",
            "echo-app-test",
        ],
        tmp.path(),
    );
    assert_success(&out, "actr init --role app");

    let app_dir = tmp.path().join("echo-app-test");
    assert!(app_dir.exists(), "echo-app-test dir should exist");
    assert_generated_actr_dependency(&app_dir);

    // 2. Verify Actr.toml declares dependency on public acme+EchoService
    let actr_toml = std::fs::read_to_string(app_dir.join("Actr.toml")).unwrap();
    assert!(
        actr_toml.contains("acme+EchoService"),
        "Actr.toml should reference acme+EchoService"
    );

    // 3. actr install: pull remote proto from public registry and install npm deps
    assert_success(
        &run_actr(&["install"], &app_dir),
        "actr install (app with public registry)",
    );

    // 4. Verify remote proto was downloaded
    let remote_proto = app_dir.join("protos/remote/echo-echo-server/echo.proto");
    assert!(
        remote_proto.exists(),
        "Remote proto should be downloaded to protos/remote/echo-echo-server/echo.proto"
    );

    // 5. actr gen -l typescript: generate client code
    assert_success(
        &run_actr(&["gen", "-l", "typescript"], &app_dir),
        "actr gen (app)",
    );

    // 6. Verify generated files
    let gen_dir = app_dir.join("src/generated");
    assert!(
        gen_dir.join("echo-echo-server/echo_pb.ts").exists(),
        "src/generated/echo-echo-server/echo_pb.ts (message types)"
    );
    assert!(
        gen_dir.join("echo-echo-server/echo_client.ts").exists(),
        "src/generated/echo-echo-server/echo_client.ts (client code for remote proto)"
    );
    assert!(
        gen_dir.join("local_actor.ts").exists(),
        "src/generated/local_actor.ts (local actor dispatcher)"
    );

    // 7. npm run typecheck: verify TypeScript compiles
    assert_success(
        &run_npm(&["run", "typecheck"], &app_dir),
        "npm run typecheck (app)",
    );

    println!("✅ App-only project with public registry dependency works correctly");
}
