use super::*;
use crate::core::DependencySpec;
use tempfile::TempDir;

const VALID_MANIFEST: &str = r#"edition = 1

[package]
name = "echo-service"
manufacturer = "acme"
version = "0.1.0"
description = "Echo service"

[dependencies]
"#;

fn manager_with(dir: &Path, body: &str) -> (TomlConfigManager, PathBuf) {
    let manifest = dir.join("manifest.toml");
    std::fs::write(&manifest, body).unwrap();
    (TomlConfigManager::new(&manifest), manifest)
}

#[tokio::test]
async fn load_config_parses_valid_manifest() {
    let dir = TempDir::new().unwrap();
    let (mgr, manifest) = manager_with(dir.path(), VALID_MANIFEST);
    let config = mgr.load_config(&manifest).await.unwrap();
    assert_eq!(config.package.name, "echo-service");
}

#[tokio::test]
async fn load_config_errors_on_missing_file() {
    let dir = TempDir::new().unwrap();
    let (mgr, _) = manager_with(dir.path(), VALID_MANIFEST);
    assert!(
        mgr.load_config(&dir.path().join("nope.toml"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn save_config_is_unsupported() {
    let dir = TempDir::new().unwrap();
    let (mgr, manifest) = manager_with(dir.path(), VALID_MANIFEST);
    let config = mgr.load_config(&manifest).await.unwrap();
    let err = mgr.save_config(&config, &manifest).await.unwrap_err();
    assert!(format!("{err}").contains("not supported"));
}

#[tokio::test]
async fn validate_config_reports_valid_and_parse_errors() {
    let dir = TempDir::new().unwrap();
    let (mgr, _) = manager_with(dir.path(), VALID_MANIFEST);
    let result = mgr.validate_config().await.unwrap();
    assert!(result.is_valid);
    assert!(result.errors.is_empty());

    // Malformed TOML → parse-error branch.
    std::fs::write(dir.path().join("manifest.toml"), "bad = {{{").unwrap();
    let result = mgr.validate_config().await.unwrap();
    assert!(!result.is_valid);
    assert!(result.errors[0].contains("Failed to parse config"));
}

#[tokio::test]
async fn validate_config_flags_empty_package_name() {
    let dir = TempDir::new().unwrap();
    let body = VALID_MANIFEST.replace("name = \"echo-service\"", "name = \"\"");
    let (mgr, _) = manager_with(dir.path(), &body);
    let result = mgr.validate_config().await.unwrap();
    // Either the parser rejects the empty name, or the validator flags it;
    // either way the config is invalid with a reported error.
    assert!(!result.is_valid);
    assert!(!result.errors.is_empty());
}

#[tokio::test]
async fn update_dependency_adds_new_entry_with_actr_type() {
    let dir = TempDir::new().unwrap();
    let (mgr, _) = manager_with(dir.path(), VALID_MANIFEST);
    let spec = DependencySpec {
        alias: "echo".into(),
        name: "echo-service".into(),
        actr_type: Some(actr_protocol::ActrType::from_string_repr("acme:Echo:1.0.0").unwrap()),
        fingerprint: Some("fp1".into()),
    };
    mgr.update_dependency(&spec).await.unwrap();
    let content = std::fs::read_to_string(dir.path().join("manifest.toml")).unwrap();
    assert!(content.contains("echo"), "added dep key: {content}");
    assert!(
        content.contains("acme:Echo:1.0.0"),
        "actr_type written: {content}"
    );
    assert!(content.contains("fp1"), "fingerprint written: {content}");
    // name differs from alias → name field emitted.
    assert!(
        content.contains("name = \"echo-service\""),
        "name written: {content}"
    );
}

#[tokio::test]
async fn update_dependency_preserves_existing_actr_type_and_fingerprint() {
    let dir = TempDir::new().unwrap();
    let body = r#"edition = 1
[package]
name = "svc"
manufacturer = "acme"
version = "0.1.0"

[dependencies]
echo = { actr_type = "acme:Echo:1.0.0", fingerprint = "keep-fp" }
"#;
    let (mgr, _) = manager_with(dir.path(), body);
    // New spec omits actr_type/fingerprint → existing must be preserved.
    let spec = DependencySpec {
        alias: "echo".into(),
        name: "echo".into(),
        actr_type: None,
        fingerprint: None,
    };
    mgr.update_dependency(&spec).await.unwrap();
    let content = std::fs::read_to_string(dir.path().join("manifest.toml")).unwrap();
    assert!(
        content.contains("acme:Echo:1.0.0"),
        "actr_type preserved: {content}"
    );
    assert!(
        content.contains("keep-fp"),
        "fingerprint preserved: {content}"
    );
}

#[tokio::test]
async fn backup_restore_remove_roundtrip() {
    let dir = TempDir::new().unwrap();
    let (mgr, manifest) = manager_with(dir.path(), VALID_MANIFEST);

    let backup = mgr.backup_config().await.unwrap();
    assert!(backup.backup_path.exists());

    // Corrupt the original, then restore from backup.
    std::fs::write(&manifest, "corrupted").unwrap();
    mgr.restore_backup(backup.clone()).await.unwrap();
    assert_eq!(std::fs::read_to_string(&manifest).unwrap(), VALID_MANIFEST);

    // Remove the backup file.
    mgr.remove_backup(backup).await.unwrap();
    // Removing again is a no-op (file gone).
    let gone = ConfigBackup {
        original_path: manifest.clone(),
        backup_path: dir.path().join("absent.bak"),
        timestamp: SystemTime::now(),
    };
    mgr.remove_backup(gone).await.unwrap();
}

#[tokio::test]
async fn backup_config_errors_when_file_missing() {
    let dir = TempDir::new().unwrap();
    let (mgr, manifest) = manager_with(dir.path(), VALID_MANIFEST);
    std::fs::remove_file(&manifest).unwrap();
    let err = mgr.backup_config().await.unwrap_err();
    assert!(format!("{err}").contains("Config file not found"));
}

#[test]
fn get_project_root_returns_parent() {
    let dir = TempDir::new().unwrap();
    let (mgr, manifest) = manager_with(dir.path(), VALID_MANIFEST);
    let expected = std::fs::canonicalize(&manifest)
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    assert_eq!(mgr.get_project_root(), expected);
}
