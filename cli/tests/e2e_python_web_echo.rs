//! End-to-end test for a generated Python EchoService workload called from actr-web.
//!
//! Run with:
//! `cargo test --test e2e_python_web_echo -- --ignored --test-threads=1`

use std::process::Command;
use std::sync::{Mutex, OnceLock};

use actr_cli::test_support::assert_success;

fn e2e_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
#[ignore] // Slow e2e; builds Python wasm, web guests, mock-actrix, and browser client.
fn python_generated_echo_workload_receives_actr_web_request() {
    let _guard = e2e_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("actr-cli should live under the workspace root");
    let script = root.join("bindings/web/examples/echo/start-python-mock.sh");
    let out = Command::new("bash")
        .arg(&script)
        .env("SUITES", "PythonWorkload")
        .env("PYTHON_ECHO_MESSAGE", "hello-from-actr-web")
        .current_dir(root.join("bindings/web/examples/echo"))
        .output()
        .expect("failed to run Python web echo e2e script");

    assert_success(&out, "bindings/web/examples/echo/start-python-mock.sh");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Python workload web echo PASSED"),
        "missing Python workload success marker:\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
