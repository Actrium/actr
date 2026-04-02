//! Integration tests for actr-cli library functionality
//!
//! These tests verify core library functions without invoking the CLI binary.

use std::fs;
use tempfile::TempDir;

#[test]
fn test_config_parser_loads_valid_config() {
    use actr::config::ConfigParser;

    let temp_dir = TempDir::new().unwrap();

    // Create a minimal valid manifest.toml (no system fields — those belong in actr.toml)
    let actr_toml = r#"edition = 1
exports = []

[package]
name = "test-service"
manufacturer = "test-company"
description = "A test service"

[dependencies]

[scripts]
dev = "cargo run"
test = "cargo test"
"#;
    let config_path = temp_dir.path().join("manifest.toml");
    fs::write(&config_path, actr_toml).unwrap();

    // Load configuration
    let config = ConfigParser::from_manifest_file(&config_path).expect("Failed to parse config");

    // Verify basic fields
    assert_eq!(config.package.name, "test-service");
    assert_eq!(config.package.actr_type.manufacturer, "test-company");
    assert_eq!(config.package.actr_type.name, "test-service");
    // ManifestConfig does not carry runtime fields (realm, signaling_url, ais_endpoint)
    // — those live in RuntimeConfig (parsed from actr.toml).

    // Verify scripts
    assert_eq!(config.scripts.get("dev"), Some(&"cargo run".to_string()));
    assert_eq!(config.scripts.get("test"), Some(&"cargo test".to_string()));
}

#[test]
fn test_template_case_conversion() {
    use actr_cli::templates::TemplateContext;

    // Test snake_case conversion
    let ctx = TemplateContext::new(
        "MyProject",
        "ws://localhost:8080",
        "acme",
        "echo-service",
        false,
    );
    assert_eq!(ctx.project_name_snake, "my_project");
    assert_eq!(ctx.project_name_pascal, "MyProject");

    // Test kebab-case conversion
    let ctx = TemplateContext::new(
        "my-project",
        "ws://localhost:8080",
        "acme",
        "echo-service",
        false,
    );
    assert_eq!(ctx.project_name_snake, "my_project");
    assert_eq!(ctx.project_name_pascal, "MyProject");

    // Test already snake_case
    let ctx = TemplateContext::new(
        "my_project",
        "ws://localhost:8080",
        "acme",
        "echo-service",
        false,
    );
    assert_eq!(ctx.project_name_snake, "my_project");
    assert_eq!(ctx.project_name_pascal, "MyProject");
}

#[test]
fn test_project_template_basic_generation() {
    use actr_cli::templates::{
        ProjectTemplate, ProjectTemplateName, SupportedLanguage, TemplateContext,
    };

    let temp_dir = TempDir::new().unwrap();

    // Load basic template
    let template = ProjectTemplate::new(ProjectTemplateName::Echo, SupportedLanguage::Rust);

    // Create template context
    let context = TemplateContext::new(
        "test-service",
        "ws://localhost:8080",
        "acme",
        "echo-service",
        false,
    );

    // Generate project files
    template
        .generate(temp_dir.path(), &context)
        .expect("Failed to generate template");

    // Verify generated files exist (Rust echo template produces main.rs, not lib.rs)
    assert!(temp_dir.path().join("Cargo.toml").exists());
    assert!(temp_dir.path().join("src/main.rs").exists());
    assert!(temp_dir.path().join("README.md").exists());

    // Verify content contains substituted project name
    let cargo_toml = fs::read_to_string(temp_dir.path().join("Cargo.toml")).unwrap();
    assert!(cargo_toml.contains("name = \"test-service\""));
}

#[tokio::test]
async fn test_start_no_record_returns_error() {
    use actr_cli::commands::{Command, start::StartCommand};
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let cmd = StartCommand {
        wid: "nonexistent00000".to_string(),
        config: None,
        hyper_dir: Some(hyper_dir.path().to_path_buf()),
    };
    let result = cmd.execute().await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("No runtime record found for WID prefix"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn test_restart_no_record_returns_error() {
    use actr_cli::commands::{Command, restart::RestartCommand};
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let cmd = RestartCommand {
        wid: "nonexistent00000".to_string(),
        config: None,
        hyper_dir: Some(hyper_dir.path().to_path_buf()),
        timeout: 5,
        force: false,
    };
    let result = cmd.execute().await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("No runtime record found for WID prefix"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn test_start_reads_config_from_runtime_record() {
    use actr_cli::commands::{Command, start::StartCommand};
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let fake_config = hyper_dir.path().join("my-custom-actr.toml");

    // With no record, the lookup should fail before the config override is used.
    let cmd = StartCommand {
        wid: "nonexistent00000".to_string(),
        config: Some(fake_config),
        hyper_dir: Some(hyper_dir.path().to_path_buf()),
    };
    let result = cmd.execute().await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("No runtime record found for WID prefix"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn test_resolve_wid_prefix_not_found() {
    use actr_cli::commands::runtime_state::RuntimeStateStore;
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let store = RuntimeStateStore::new(hyper_dir.path().to_path_buf());
    let result = store.resolve_wid_prefix("abcdef1234567890").await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("No runtime record found for WID prefix"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn test_resolve_wid_prefix_short_rejected() {
    use actr_cli::commands::runtime_state::RuntimeStateStore;
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let store = RuntimeStateStore::new(hyper_dir.path().to_path_buf());
    let result = store.resolve_wid_prefix("abcdefg").await; // 7 chars
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("at least 8 characters"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn test_resolve_wid_prefix_exact_match() {
    use actr_cli::commands::runtime_state::{RuntimeRecord, RuntimeStateStore};
    use chrono::Utc;
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let store = RuntimeStateStore::new(hyper_dir.path().to_path_buf());
    store.ensure_layout().await.unwrap();

    let wid = "aaaabbbbccccdddd-0000-0000-0000-000000000000".to_string();
    let record = RuntimeRecord::new(
        wid.clone(),
        "test-actr-id".to_string(),
        99999,
        hyper_dir.path().join("actr.toml"),
        hyper_dir.path().join("logs").join("actr-test.log"),
        Utc::now(),
    );
    store.write_record(&record).await.unwrap();

    let entry = store.resolve_wid_prefix(&wid).await.unwrap();
    assert_eq!(entry.record.wid, wid);
    assert_eq!(entry.record.actr_id, "test-actr-id");
}

#[tokio::test]
async fn test_resolve_wid_prefix_ambiguous() {
    use actr_cli::commands::runtime_state::{RuntimeRecord, RuntimeStateStore};
    use chrono::Utc;
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let store = RuntimeStateStore::new(hyper_dir.path().to_path_buf());
    store.ensure_layout().await.unwrap();

    // Two wids sharing the same 8-char prefix "aaaabbbb"
    let wid1 = "aaaabbbb-1111-1111-1111-111111111111".to_string();
    let wid2 = "aaaabbbb-2222-2222-2222-222222222222".to_string();

    for (wid, actr_id) in [(&wid1, "actr-1"), (&wid2, "actr-2")] {
        let record = RuntimeRecord::new(
            wid.clone(),
            actr_id.to_string(),
            99999,
            hyper_dir.path().join("actr.toml"),
            hyper_dir.path().join("logs").join("actr-test.log"),
            Utc::now(),
        );
        store.write_record(&record).await.unwrap();
    }

    let result = store.resolve_wid_prefix("aaaabbbb").await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("Ambiguous WID prefix"),
        "unexpected error: {msg}"
    );
    assert!(
        msg.contains("actr-1") || msg.contains("actr-2"),
        "candidates missing: {msg}"
    );
}

#[tokio::test]
async fn test_resolve_wid_prefix_ambiguous_with_short_wid() {
    use actr_cli::commands::runtime_state::{RuntimeRecord, RuntimeStateStore};
    use chrono::Utc;
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let store = RuntimeStateStore::new(hyper_dir.path().to_path_buf());
    store.ensure_layout().await.unwrap();

    let wid1 = "aaaabbbb1".to_string();
    let wid2 = "aaaabbbb2".to_string();

    for (wid, actr_id) in [(&wid1, "actr-short-1"), (&wid2, "actr-short-2")] {
        let record = RuntimeRecord::new(
            wid.clone(),
            actr_id.to_string(),
            99999,
            hyper_dir.path().join("actr.toml"),
            hyper_dir.path().join("logs").join("actr-test.log"),
            Utc::now(),
        );
        store.write_record(&record).await.unwrap();
    }

    let result = store.resolve_wid_prefix("aaaabbbb").await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("aaaabbbb1") && msg.contains("aaaabbbb2"),
        "short wid candidates missing: {msg}"
    );
}

#[tokio::test]
async fn test_upsert_record_updates_pid() {
    use actr_cli::commands::runtime_state::{RuntimeRecord, RuntimeStateStore};
    use chrono::Utc;
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let store = RuntimeStateStore::new(hyper_dir.path().to_path_buf());
    store.ensure_layout().await.unwrap();

    let wid = "upsert00-0000-0000-0000-000000000000".to_string();
    let record1 = RuntimeRecord::new(
        wid.clone(),
        "test-actr".to_string(),
        100,
        hyper_dir.path().join("actr.toml"),
        hyper_dir.path().join("logs").join("actr-test.log"),
        Utc::now(),
    );
    store.write_record(&record1).await.unwrap();

    // Overwrite with updated pid
    let mut record2 = record1.clone();
    record2.pid = 200;
    record2.stopped_at = None;
    store.write_record(&record2).await.unwrap();

    let result = store.read_record_by_wid(&wid).await.unwrap().unwrap();
    assert_eq!(result.pid, 200);
    assert_eq!(result.wid, wid);
    assert!(result.stopped_at.is_none());
}

#[tokio::test]
async fn test_mark_stopped_by_wid() {
    use actr_cli::commands::runtime_state::{RuntimeRecord, RuntimeStateStore};
    use chrono::Utc;
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let store = RuntimeStateStore::new(hyper_dir.path().to_path_buf());
    store.ensure_layout().await.unwrap();

    let wid = "stoptest-0000-0000-0000-000000000000".to_string();
    let record = RuntimeRecord::new(
        wid.clone(),
        "test-actr".to_string(),
        99999,
        hyper_dir.path().join("actr.toml"),
        hyper_dir.path().join("logs").join("actr-test.log"),
        Utc::now(),
    );
    store.write_record(&record).await.unwrap();

    let stopped_at = Utc::now();
    store.mark_stopped_by_wid(&wid, stopped_at).await.unwrap();

    let result = store.read_record_by_wid(&wid).await.unwrap().unwrap();
    assert!(result.stopped_at.is_some());
}

#[tokio::test]
async fn test_schema_v1_record_returns_error() {
    use actr_cli::commands::runtime_state::RuntimeStateStore;
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let store = RuntimeStateStore::new(hyper_dir.path().to_path_buf());
    store.ensure_layout().await.unwrap();

    // Write a v1-style record (no wid field)
    let v1_json = r#"{"schema_version":1,"actr_id":"old-id","pid":12345,"config_path":"/tmp/actr.toml","log_path":"/tmp/actr.log","started_at":"2024-01-01T00:00:00Z","stopped_at":null}"#;
    let record_path = store.run_dir().join("12345.json");
    tokio::fs::write(&record_path, v1_json).await.unwrap();

    let result = store.list_records().await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("Incompatible runtime record schema"),
        "unexpected error: {msg}"
    );
    // Error message should include the run_dir path for easy remediation
    assert!(
        msg.contains(store.run_dir().to_str().unwrap()),
        "run_dir path missing from error: {msg}"
    );
}

#[tokio::test]
async fn test_rm_removes_stopped_record() {
    use actr_cli::commands::runtime_state::{RuntimeRecord, RuntimeStateStore};
    use actr_cli::commands::{Command, rm::RmCommand};
    use chrono::Utc;
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let store = RuntimeStateStore::new(hyper_dir.path().to_path_buf());
    store.ensure_layout().await.unwrap();

    let wid = "rmtest00-0000-0000-0000-000000000000".to_string();
    let mut record = RuntimeRecord::new(
        wid.clone(),
        "test-actr".to_string(),
        99999,
        hyper_dir.path().join("actr.toml"),
        hyper_dir.path().join("logs").join("actr-test.log"),
        Utc::now(),
    );
    record.stopped_at = Some(Utc::now());
    store.write_record(&record).await.unwrap();

    let cmd = RmCommand {
        wid: "rmtest00".to_string(),
        config: None,
        hyper_dir: Some(hyper_dir.path().to_path_buf()),
        force: false,
    };
    cmd.execute().await.unwrap();

    assert!(store.read_record_by_wid(&wid).await.unwrap().is_none());
}

#[tokio::test]
async fn test_rm_rejects_running_record_without_force() {
    use actr_cli::commands::runtime_state::{RuntimeRecord, RuntimeStateStore};
    use actr_cli::commands::{Command, rm::RmCommand};
    use chrono::Utc;
    use tempfile::TempDir;

    let hyper_dir = TempDir::new().unwrap();
    let store = RuntimeStateStore::new(hyper_dir.path().to_path_buf());
    store.ensure_layout().await.unwrap();

    let wid = "rmalive0-0000-0000-0000-000000000000".to_string();
    let record = RuntimeRecord::new(
        wid,
        "test-actr".to_string(),
        std::process::id(),
        hyper_dir.path().join("actr.toml"),
        hyper_dir.path().join("logs").join("actr-test.log"),
        Utc::now(),
    );
    store.write_record(&record).await.unwrap();

    let cmd = RmCommand {
        wid: "rmalive0".to_string(),
        config: None,
        hyper_dir: Some(hyper_dir.path().to_path_buf()),
        force: false,
    };
    let result = cmd.execute().await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("Stop it first or use -f"),
        "unexpected error: {msg}"
    );
}
