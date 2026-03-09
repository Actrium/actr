use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn actr_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_actr"))
}

fn run_actr(args: &[&str], cwd: &Path) -> Output {
    Command::new(actr_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run actr binary")
}

fn write_minimal_actr_files(root: &Path) {
    let actr_toml = r#"edition = 1
exports = []

[package]
name = "demo-project"
description = "Test project"

[package.actr_type]
manufacturer = "acme"
name = "demo-service"

[dependencies]

[system.signaling]
url = "ws://localhost:8080"

[system.deployment]
realm_id = 1001

[system.discovery]
visible = true

[scripts]
dev = "echo dev"
"#;

    fs::write(root.join("actr.toml"), actr_toml).unwrap();
    fs::write(root.join("Actr.lock.toml"), "[metadata]\nversion = 1\n").unwrap();
}

#[test]
fn typescript_project_rejects_rust_codegen() {
    let tmp = TempDir::new().unwrap();
    write_minimal_actr_files(tmp.path());
    fs::write(
        tmp.path().join("tsconfig.json"),
        "{\n  \"compilerOptions\": {}\n}\n",
    )
    .unwrap();
    fs::write(
        tmp.path().join("package.json"),
        "{\n  \"name\": \"demo\"\n}\n",
    )
    .unwrap();

    let out = run_actr(&["gen", "-l", "rust"], tmp.path());
    assert!(
        !out.status.success(),
        "TypeScript project should reject rust codegen"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Refusing to generate 'rust' code in a 'typescript' project"),
        "stderr should explain the mismatch, got:\n{stderr}"
    );
    assert!(
        stderr.contains("actr gen -l typescript"),
        "stderr should suggest the matching command, got:\n{stderr}"
    );
}

#[test]
fn rust_project_rejects_typescript_codegen() {
    let tmp = TempDir::new().unwrap();
    write_minimal_actr_files(tmp.path());
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"demo-project\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();

    let out = run_actr(&["gen", "-l", "typescript"], tmp.path());
    assert!(
        !out.status.success(),
        "Rust project should reject typescript codegen"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Refusing to generate 'typescript' code in a 'rust' project"),
        "stderr should explain the mismatch, got:\n{stderr}"
    );
    assert!(
        stderr.contains("actr gen -l rust"),
        "stderr should suggest the matching command, got:\n{stderr}"
    );
}

#[test]
fn unknown_project_prints_warning_before_continuing() {
    let tmp = TempDir::new().unwrap();
    write_minimal_actr_files(tmp.path());

    let out = run_actr(&["gen", "-l", "rust"], tmp.path());
    assert!(
        !out.status.success(),
        "Unknown project should continue past the language guard and fail later"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Warning: Could not detect project language"),
        "stderr should include the warning, got:\n{stderr}"
    );
    assert!(
        !stderr.contains("Refusing to generate"),
        "Unknown project should not be rejected by the language guard, got:\n{stderr}"
    );
}

#[test]
fn ambiguous_project_prints_warning_before_continuing() {
    let tmp = TempDir::new().unwrap();
    write_minimal_actr_files(tmp.path());
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"demo-project\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(
        tmp.path().join("package.json"),
        "{\n  \"name\": \"demo\"\n}\n",
    )
    .unwrap();

    let out = run_actr(&["gen", "-l", "rust"], tmp.path());
    assert!(
        !out.status.success(),
        "Ambiguous project should continue past the language guard and fail later"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Warning: Detected multiple project language markers"),
        "stderr should include the warning, got:\n{stderr}"
    );
    assert!(
        !stderr.contains("Refusing to generate"),
        "Ambiguous project should not be rejected by the language guard, got:\n{stderr}"
    );
}
