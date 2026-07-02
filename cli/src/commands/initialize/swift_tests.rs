use super::*;
use tempfile::TempDir;

#[test]
fn create_swiftpm_registry_config_writes_expected_file() {
    let dir = TempDir::new().unwrap();
    create_swiftpm_registry_config(dir.path()).unwrap();
    let config = dir.path().join(".swiftpm/configuration/registries.json");
    assert!(config.exists());
    let content = std::fs::read_to_string(&config).unwrap();
    assert!(content.contains("tuist.dev"));
    assert!(content.contains("apple"));
}
