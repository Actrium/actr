//! Integration tests for actr-cli library functionality
//!
//! These tests verify core library functions without invoking the CLI binary.

use std::fs;
use tempfile::TempDir;

#[test]
fn test_config_parser_loads_valid_config() {
    use actr::config::ConfigParser;

    let temp_dir = TempDir::new().unwrap();

    // Create a minimal valid manifest.toml
    let actr_toml = r#"edition = 1
exports = []

[package]
name = "test-service"
manufacturer = "test-company"
description = "A test service"

[dependencies]

[system.signaling]
url = "ws://localhost:8080/"

[system.ais_endpoint]
url = "http://localhost:8080/ais"

[system.deployment]
realm_id = 1001

[system.discovery]
visible = true

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
    assert_eq!(config.realm.realm_id, 1001);
    assert!(config.visible_in_discovery);

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
