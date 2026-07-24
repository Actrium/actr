use super::*;
use tempfile::TempDir;

#[test]
fn test_template_context() {
    let ctx = TemplateContext::new(
        "my-chat-service",
        "ws://localhost:8080",
        DEFAULT_MANUFACTURER,
        "echo-service",
        false,
    );
    assert_eq!(ctx.project_name, "my-chat-service");
    assert_eq!(ctx.project_name_snake, "my_chat_service");
    assert_eq!(ctx.project_name_pascal, "MyChatService");
    assert_eq!(ctx.workload_name, "MyChatServiceWorkload");
    assert_eq!(ctx.signaling_url, "ws://localhost:8080");
    assert_eq!(ctx.ais_endpoint_url, "http://localhost:8080/ais");
    assert_eq!(ctx.actr_swift_version, DEFAULT_ACTR_SWIFT_VERSION);
    assert_eq!(ctx.actr_workload_version, DEFAULT_ACTR_WORKLOAD_VERSION);
}

#[test]
fn test_project_template_new() {
    let template = ProjectTemplate::new(ProjectTemplateName::Echo, SupportedLanguage::Swift);
    assert_eq!(template.name, ProjectTemplateName::Echo);
}

#[test]
fn test_project_template_generation() {
    let temp_dir = TempDir::new().unwrap();
    let template = ProjectTemplate::new(ProjectTemplateName::Echo, SupportedLanguage::Swift);
    let context = TemplateContext::new(
        "test-app",
        "ws://localhost:8080",
        DEFAULT_MANUFACTURER,
        "echo-service",
        false,
    );

    template
        .generate(temp_dir.path(), &context)
        .expect("Failed to generate");

    // Verify project.yml exists
    assert!(temp_dir.path().join("project.yml").exists());
    // Verify manifest.toml exists
    assert!(temp_dir.path().join("manifest.toml").exists());
    // Verify .gitignore exists
    assert!(temp_dir.path().join(".gitignore").exists());
    // Note: proto files are no longer created during init, they will be pulled via actr deps install
    // Verify app directory exists
    assert!(
        temp_dir
            .path()
            .join("TestApp")
            .join("TestApp.swift")
            .exists()
    );
}

#[test]
fn test_project_template_load_files() {
    let template = ProjectTemplate::new(ProjectTemplateName::Echo, SupportedLanguage::Swift);
    let context = TemplateContext::new(
        "test-app",
        "ws://localhost:8080",
        DEFAULT_MANUFACTURER,
        "echo-service",
        false,
    );
    let result = template
        .lang_template
        .load_files(ProjectTemplateName::Echo, &context);
    assert!(result.is_ok());
}
