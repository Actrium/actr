//! Integration coverage for low-dependency CLI command paths.
//!
//! These tests intentionally exercise the real `actr` binary from isolated
//! temporary directories so clap dispatch, filesystem IO, and rendered command
//! results are all covered without relying on network services.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

fn actr_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_actr"))
}

fn run_actr(args: &[&str], cwd: &Path, home: &Path) -> Output {
    Command::new(actr_bin())
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("NO_COLOR", "1")
        .env("RUST_LOG", "off")
        .output()
        .expect("failed to run actr binary")
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure(output: &Output, context: &str) {
    assert!(
        !output.status.success(),
        "{context} unexpectedly succeeded:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn clean_stdout(output: &Output) -> String {
    strip_ansi(&stdout(output))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn strip_ansi(input: &str) -> String {
    let mut clean = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        clean.push(ch);
    }
    clean
}

fn first_json_object(output: &Output) -> serde_json::Value {
    let text = stdout(output);
    let start = text.find('{').expect("stdout should contain a JSON object");
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    return serde_json::from_str(&text[start..end])
                        .expect("stdout JSON object should parse");
                }
            }
            _ => {}
        }
    }

    panic!("stdout JSON object did not terminate: {text}");
}

fn isolated_home(root: &Path) -> PathBuf {
    let home = root.join("home");
    fs::create_dir_all(&home).expect("create isolated home");
    home
}

fn write_manifest_with_proto(root: &Path) {
    fs::create_dir_all(root.join("proto")).expect("create proto dir");
    fs::write(
        root.join("proto/echo.proto"),
        r#"syntax = "proto3";
package echo;

service EchoService {
  rpc Echo (EchoRequest) returns (EchoReply);
}

message EchoRequest {
  string message = 1;
}

message EchoReply {
  string message = 1;
}
"#,
    )
    .expect("write proto");

    fs::write(
        root.join("manifest.toml"),
        r#"edition = 1
exports = ["proto/echo.proto"]

[package]
name = "echo-service"
manufacturer = "acme"
version = "0.1.0"
description = "Echo service"

[dependencies]
"#,
    )
    .expect("write manifest");
}

#[test]
fn config_local_scope_round_trips_values_and_validates() {
    let tmp = TempDir::new().expect("tempdir");
    let home = isolated_home(tmp.path());

    let set = run_actr(
        &["config", "--local", "set", "network.realm_id", "4242"],
        tmp.path(),
        &home,
    );
    assert_success(&set, "config local set");
    assert!(clean_stdout(&set).contains("Updated local config"));

    let config_path = tmp.path().join(".actr/config.toml");
    let saved = fs::read_to_string(&config_path).expect("read local config");
    assert!(saved.contains("realm_id = 4242"), "saved config:\n{saved}");

    let get = run_actr(
        &["config", "--local", "get", "network.realm_id"],
        tmp.path(),
        &home,
    );
    assert_success(&get, "config local get");
    assert_eq!(clean_stdout(&get).trim(), "4242");

    let show = run_actr(
        &["config", "--local", "show", "--format", "json"],
        tmp.path(),
        &home,
    );
    assert_success(&show, "config local show json");
    let show_json: serde_json::Value =
        serde_json::from_str(clean_stdout(&show).trim()).expect("show output should be JSON");
    assert_eq!(show_json["network"]["realm_id"], 4242);

    let test = run_actr(&["config", "--local", "test"], tmp.path(), &home);
    assert_success(&test, "config local test");
    assert!(clean_stdout(&test).contains("Local config syntax and schema are valid"));

    let unset = run_actr(
        &["config", "--local", "unset", "network.realm_id"],
        tmp.path(),
        &home,
    );
    assert_success(&unset, "config local unset");

    let get_missing = run_actr(
        &["config", "--local", "get", "network.realm_id"],
        tmp.path(),
        &home,
    );
    assert_failure(&get_missing, "config local get after unset");
    assert!(stderr(&get_missing).contains("Configuration key 'network.realm_id' not found"));
}

#[test]
fn registry_fingerprint_reports_service_json_and_lock_mismatches() {
    let tmp = TempDir::new().expect("tempdir");
    let home = isolated_home(tmp.path());
    write_manifest_with_proto(tmp.path());

    let output = run_actr(
        &[
            "registry",
            "fingerprint",
            "--manifest-path",
            "manifest.toml",
            "--format",
            "json",
        ],
        tmp.path(),
        &home,
    );
    assert_success(&output, "registry fingerprint json");
    let json = first_json_object(&output);
    assert_eq!(json["proto_files"][0], "echo.proto");
    assert_eq!(json["verification"]["status"], "not_requested");

    fs::write(
        tmp.path().join("manifest.lock.toml"),
        r#"[[dependency]]
name = "remote-echo"
fingerprint = "service_semantic:stale"

[[dependency.files]]
path = "proto/echo.proto"
fingerprint = "semantic:stale"
"#,
    )
    .expect("write lock");

    let verify = run_actr(
        &[
            "registry",
            "fingerprint",
            "--manifest-path",
            "manifest.toml",
            "--format",
            "json",
            "--verify",
        ],
        tmp.path(),
        &home,
    );
    assert_success(&verify, "registry fingerprint verify json");
    let verify_json = first_json_object(&verify);
    assert_eq!(verify_json["verification"]["status"], "failed");
    let mismatches = verify_json["verification"]["mismatches"]
        .as_array()
        .expect("mismatches should be an array");
    assert!(
        mismatches
            .iter()
            .any(|item| item["file_path"] == "proto/echo.proto")
    );
    assert!(
        mismatches
            .iter()
            .any(|item| item["file_path"] == "SERVICE_FINGERPRINT")
    );
}

#[test]
fn registry_fingerprint_supports_proto_yaml_and_missing_proto_errors() {
    let tmp = TempDir::new().expect("tempdir");
    let home = isolated_home(tmp.path());
    write_manifest_with_proto(tmp.path());

    let output = run_actr(
        &[
            "registry",
            "fingerprint",
            "--proto",
            "proto/echo.proto",
            "--format",
            "yaml",
        ],
        tmp.path(),
        &home,
    );
    assert_success(&output, "registry fingerprint proto yaml");
    let yaml = clean_stdout(&output);
    assert!(
        yaml.contains("proto_file: proto/echo.proto"),
        "yaml:\n{yaml}"
    );
    assert!(yaml.contains("fingerprint:"), "yaml:\n{yaml}");

    let missing = run_actr(
        &["registry", "fingerprint", "--proto", "proto/missing.proto"],
        tmp.path(),
        &home,
    );
    assert_failure(&missing, "registry fingerprint missing proto");
    assert!(stderr(&missing).contains("Proto file not found: proto/missing.proto"));
}

#[test]
fn doc_generates_static_pages_from_manifest_and_proto_tree() {
    let tmp = TempDir::new().expect("tempdir");
    let home = isolated_home(tmp.path());
    write_manifest_with_proto(tmp.path());
    fs::create_dir_all(tmp.path().join("protos/local")).expect("create doc proto dir");
    fs::copy(
        tmp.path().join("proto/echo.proto"),
        tmp.path().join("protos/local/echo.proto"),
    )
    .expect("copy doc proto");

    let output = run_actr(&["doc", "--output", "docs-out"], tmp.path(), &home);
    assert_success(&output, "doc generation");

    for page in ["index.html", "api.html", "config.html"] {
        let path = tmp.path().join("docs-out").join(page);
        assert!(path.exists(), "expected generated page: {}", path.display());
    }

    let api = fs::read_to_string(tmp.path().join("docs-out/api.html")).expect("read api page");
    assert!(api.contains("EchoService"), "api page:\n{api}");
}

#[test]
fn pkg_keygen_writes_key_config_and_rejects_existing_key_without_force() {
    let tmp = TempDir::new().expect("tempdir");
    let home = isolated_home(tmp.path());
    let key_path = tmp.path().join("keys/dev-key.json");
    let key_arg = key_path.to_string_lossy().into_owned();

    let output = run_actr(&["pkg", "keygen", "--output", &key_arg], tmp.path(), &home);
    assert_success(&output, "pkg keygen");
    assert!(key_path.exists(), "key was not written");

    let key_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&key_path).expect("read key"))
            .expect("key should be JSON");
    assert!(key_json["private_key"].as_str().is_some());
    assert!(key_json["public_key"].as_str().is_some());

    let global_config =
        fs::read_to_string(home.join(".actr/config.toml")).expect("read global config");
    assert!(
        global_config.contains("keychain"),
        "global config:\n{global_config}"
    );
    assert!(
        global_config.contains(&key_arg),
        "global config:\n{global_config}"
    );

    let duplicate = run_actr(&["pkg", "keygen", "--output", &key_arg], tmp.path(), &home);
    assert_failure(&duplicate, "duplicate pkg keygen");
    assert!(stderr(&duplicate).contains("Key file already exists"));

    let force = run_actr(
        &["pkg", "keygen", "--output", &key_arg, "--force"],
        tmp.path(),
        &home,
    );
    assert_success(&force, "forced pkg keygen");
}
