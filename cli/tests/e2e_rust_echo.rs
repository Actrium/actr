//! End-to-end tests for Rust echo template
//!
//! These tests build and run real service + app communication over the signaling server.
//! They are slow (~200s) and require network access.
//!
//! Run with: `cargo test --test e2e_rust_echo -- --ignored --test-threads=1`

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
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

fn random_manufacturer() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("test{nanos:08x}")
}

fn cargo_build(dir: &std::path::Path) {
    let out = Command::new("cargo")
        .args(["build"])
        .current_dir(dir)
        .output()
        .expect("cargo build failed");
    assert!(
        out.status.success(),
        "cargo build failed in {}:\nstderr: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Wait until the child writes a line containing `needle` to stdout/stderr,
/// or until timeout. Returns `(found, collected_lines)`.
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
    while !found.load(Ordering::SeqCst) {
        if std::time::Instant::now() > deadline {
            return (false, lines);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    (true, lines)
}

/// End-to-end: generate both projects via `--role both`, build, start service,
/// run app, verify round-trip echo reply.
#[test]
#[ignore] // Slow test (~200s), run explicitly with --ignored
fn rust_echo_e2e_service_and_app() {
    let tmp = TempDir::new().unwrap();
    let mfr = random_manufacturer();

    // 1. actr init --role both
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
            &mfr,
            "e2e",
        ],
        tmp.path(),
    );
    assert_actr_success(&out, "actr init --role both");

    let svc_dir = tmp.path().join("e2e/echo-service");
    let app_dir = tmp.path().join("e2e/echo-app");
    assert!(svc_dir.exists(), "echo-service dir");
    assert!(app_dir.exists(), "echo-app dir");

    // 2. Build service: install → gen → cargo build
    assert_actr_success(&run_actr(&["install"], &svc_dir), "actr install (svc)");
    assert_actr_success(
        &run_actr(&["gen", "-l", "rust"], &svc_dir),
        "actr gen (svc)",
    );
    cargo_build(&svc_dir);

    // 3. Start service, wait for registration on signaling server
    let mut svc = Command::new("cargo")
        .args(["run"])
        .current_dir(&svc_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let (ready, svc_log) = wait_for_output(
        &mut svc,
        "EchoService registered",
        std::time::Duration::from_secs(120),
    );
    assert!(ready, "service not ready within 120s (mfr={mfr})");

    // 4. Build app: service is live on signaling, so `actr install` can discover it
    assert_actr_success(&run_actr(&["install"], &app_dir), "actr install (app)");
    assert_actr_success(
        &run_actr(&["gen", "-l", "rust"], &app_dir),
        "actr gen (app)",
    );
    cargo_build(&app_dir);

    // 5. Run app, wait for exit
    let mut app = Command::new("cargo")
        .args(["run"])
        .current_dir(&app_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match app.try_wait().unwrap() {
            Some(_) => break,
            None if std::time::Instant::now() > deadline => {
                app.kill().ok();
                svc.kill().ok();
                panic!("app did not exit within 60s");
            }
            _ => std::thread::sleep(std::time::Duration::from_millis(500)),
        }
    }

    let app_out = app.wait_with_output().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let svc_output = svc_log.lock().unwrap().join("\n");
    svc.kill().ok();
    svc.wait().ok();

    let stdout = String::from_utf8_lossy(&app_out.stdout);
    let stderr = String::from_utf8_lossy(&app_out.stderr);
    assert!(
        app_out.status.success(),
        "app failed:\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("Echo reply:"),
        "missing echo reply in:\n{stdout}"
    );
    assert!(
        svc_output.contains("Received echo request: hello"),
        "service missing request log in:\n{svc_output}"
    );
}

/// End-to-end: generate app-only project that depends on public registry `acme+EchoService`.
/// Verifies that `actr install` can pull remote proto and `actr gen` generates client code.
#[test]
#[ignore] // Requires public registry access
fn rust_echo_e2e_app_with_public_registry() {
    let tmp = TempDir::new().unwrap();

    // 1. actr init --role app (uses default manufacturer 'acme')
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
            "echo-app-test",
        ],
        tmp.path(),
    );
    assert_actr_success(&out, "actr init --role app");

    let app_dir = tmp.path().join("echo-app-test");
    assert!(app_dir.exists(), "echo-app-test dir");

    // 2. Verify Actr.toml declares dependency on public acme+EchoService
    let actr_toml = std::fs::read_to_string(app_dir.join("Actr.toml")).unwrap();
    assert!(
        actr_toml.contains("acme+EchoService"),
        "Actr.toml should reference acme+EchoService"
    );

    // 3. actr install: pull remote proto from public registry
    let out = run_actr(&["install"], &app_dir);
    assert_actr_success(&out, "actr install (app with public registry)");

    // 4. Verify remote proto was downloaded
    let remote_proto = app_dir.join("protos/remote/echo-echo-server/echo.proto");
    assert!(
        remote_proto.exists(),
        "Remote proto should be downloaded to protos/remote/echo-echo-server/echo.proto"
    );

    // 5. actr gen -l rust: generate client code
    let out = run_actr(&["gen", "-l", "rust"], &app_dir);
    assert_actr_success(&out, "actr gen (app)");

    // 6. Verify generated files
    let gen_dir = app_dir.join("src/generated");
    assert!(gen_dir.join("mod.rs").exists(), "generated/mod.rs");
    assert!(
        gen_dir.join("echo.rs").exists(),
        "generated/echo.rs (message types)"
    );
    assert!(
        gen_dir.join("echo_client.rs").exists(),
        "generated/echo_client.rs (client code for remote proto)"
    );

    // 7. Verify mod.rs declares both modules
    let mod_rs = std::fs::read_to_string(gen_dir.join("mod.rs")).unwrap();
    assert!(
        mod_rs.contains("pub mod echo;"),
        "mod.rs should declare echo module"
    );
    assert!(
        mod_rs.contains("pub mod echo_client;"),
        "mod.rs should declare echo_client module"
    );

    // 8. cargo build: verify project compiles
    cargo_build(&app_dir);

    println!("✅ App-only project with public registry dependency works correctly");
}
