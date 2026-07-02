use super::DetectedProjectLanguage;
use std::fs;
use tempfile::TempDir;

#[test]
fn detects_rust_from_cargo_toml() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .unwrap();

    assert_eq!(
        DetectedProjectLanguage::detect(dir.path()),
        DetectedProjectLanguage::Rust
    );
}

#[test]
fn returns_unknown_without_language_markers() {
    let dir = TempDir::new().unwrap();

    assert_eq!(
        DetectedProjectLanguage::detect(dir.path()),
        DetectedProjectLanguage::Unknown
    );
}

#[test]
fn returns_ambiguous_with_multiple_language_markers() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"demo\"\n}\n",
    )
    .unwrap();

    assert_eq!(
        DetectedProjectLanguage::detect(dir.path()),
        DetectedProjectLanguage::Ambiguous
    );
}
