use super::*;
use tempfile::TempDir;

#[test]
fn test_command_exists() {
    // These commands should exist on most systems
    assert!(command_exists("ls") || command_exists("dir"));
    assert!(!command_exists("this_command_definitely_does_not_exist"));
}

#[test]
fn test_ensure_dir_exists() {
    let temp_dir = TempDir::new().unwrap();
    let test_path = temp_dir.path().join("test/nested/dir");

    assert!(!test_path.exists());
    ensure_dir_exists(&test_path).unwrap();
    assert!(test_path.exists());

    // Should not fail if directory already exists
    ensure_dir_exists(&test_path).unwrap();
}

#[tokio::test]
async fn test_execute_command() {
    // Test a simple command that should succeed
    let result = execute_command("echo", &["hello"], None).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello"));
}

#[test]
fn case_conversions_round_trip() {
    assert_eq!(to_pascal_case("echo_service"), "EchoService");
    assert_eq!(to_snake_case("EchoService"), "echo_service");
}

#[test]
fn copy_file_with_dirs_creates_parent_directories() {
    let temp_dir = TempDir::new().unwrap();
    let src = temp_dir.path().join("src.txt");
    std::fs::write(&src, "payload").unwrap();
    let dst = temp_dir.path().join("nested/deep/dst.txt");
    copy_file_with_dirs(&src, &dst).unwrap();
    assert_eq!(std::fs::read_to_string(&dst).unwrap(), "payload");
}
