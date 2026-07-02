use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn discover_proto_files_finds_protos_in_dir() {
    let processor = DefaultProtoProcessor::new();
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.proto"), "syntax = \"proto3\";").unwrap();
    std::fs::write(dir.path().join("b.proto"), "message X {}").unwrap();
    std::fs::write(dir.path().join("readme.txt"), "not proto").unwrap();
    let mut files = processor.discover_proto_files(dir.path()).await.unwrap();
    files.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(files.len(), 2);
    assert!(files[0].name.ends_with(".proto"));
    // Non-dir path returns empty (no read_dir).
    let empty = processor
        .discover_proto_files(&dir.path().join("a.proto"))
        .await
        .unwrap();
    assert!(empty.is_empty());
}

#[tokio::test]
async fn parse_proto_services_is_a_stub() {
    let processor = DefaultProtoProcessor::new();
    let result = processor.parse_proto_services(&[]).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn generate_code_returns_output_path_in_generation_result() {
    let processor = DefaultProtoProcessor::new();
    let output = std::path::Path::new("/tmp/out");
    let genres = processor
        .generate_code(std::path::Path::new("/in"), output)
        .await
        .unwrap();
    assert_eq!(genres.generated_files, vec![output.to_path_buf()]);
    assert!(genres.warnings.is_empty());
    assert!(genres.errors.is_empty());
}

#[tokio::test]
async fn validate_proto_syntax_returns_clean_report() {
    let processor = DefaultProtoProcessor::new();
    let report = processor.validate_proto_syntax(&[]).await.unwrap();
    assert!(report.is_valid);
    assert!(report.config_validation.is_valid);
    assert!(report.is_success());
}
