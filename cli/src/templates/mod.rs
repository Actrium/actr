//! Project template system

pub mod kotlin;
pub mod python;
pub mod rust;
pub mod swift;
pub mod typescript;
pub mod web;

use self::kotlin::KotlinTemplate;
use self::python::PythonTemplate;
use self::rust::RustTemplate;
use self::swift::SwiftTemplate;
use self::typescript::TypeScriptTemplate;
use crate::assets::FixtureAssets;
use crate::error::{ActrCliError, Result};
use crate::utils::{to_pascal_case, to_snake_case};
use clap::ValueEnum;
use handlebars::Handlebars;
use serde::Serialize;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub use crate::commands::SupportedLanguage;

pub const DEFAULT_ACTR_SWIFT_VERSION: &str = "0.1.15";
pub const DEFAULT_ACTR_PROTOCOLS_VERSION: &str = "0.1.2";
pub const DEFAULT_MANUFACTURER: &str = "acme";

/// Project template options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum, Serialize)]
#[value(rename_all = "lowercase")]
pub enum ProjectTemplateName {
    #[default]
    Echo,
    #[value(name = "data-stream")]
    DataStream,
}

impl ProjectTemplateName {
    /// Maps template name to remote service name
    pub fn to_service_name(self) -> &'static str {
        match self {
            ProjectTemplateName::Echo => "echo-service",
            ProjectTemplateName::DataStream => "data-stream-service",
        }
    }
}

impl std::fmt::Display for ProjectTemplateName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pv = self
            .to_possible_value()
            .expect("ValueEnum variant must have a possible value");
        write!(f, "{}", pv.get_name())
    }
}

/// Role for the echo template: service (provides EchoService), app (calls EchoService),
/// or both (generate app and service projects).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum, Serialize)]
#[value(rename_all = "lowercase")]
pub enum EchoRole {
    /// Provides EchoService, waits for RPC calls.
    Service,
    /// Calls EchoService, sends echo RPC and exits.
    #[default]
    App,
    /// Generates both EchoService provider and app projects.
    Both,
}

#[derive(Debug, Clone, Serialize)]
pub struct TemplateContext {
    #[serde(rename = "PROJECT_NAME")]
    pub project_name: String,
    #[serde(rename = "PROJECT_NAME_SNAKE")]
    pub project_name_snake: String,
    #[serde(rename = "PROJECT_NAME_PASCAL")]
    pub project_name_pascal: String,
    #[serde(rename = "SIGNALING_URL")]
    pub signaling_url: String,
    #[serde(rename = "MANUFACTURER")]
    pub manufacturer: String,
    #[serde(rename = "SERVICE_NAME")]
    pub service_name: String,
    #[serde(rename = "WORKLOAD_NAME")]
    pub workload_name: String,
    #[serde(rename = "ACTR_SWIFT_VERSION")]
    pub actr_swift_version: String,
    #[serde(rename = "ACTR_PROTOCOLS_VERSION")]
    pub actr_protocols_version: String,
    #[serde(rename = "ACTR_LOCAL_PATH")]
    pub actr_local_path: Option<String>,
    #[serde(rename = "ACTR_WEB_LOCAL_PATH")]
    pub actr_web_local_path: Option<String>,
    #[serde(rename = "REALM_ID")]
    pub realm_id: u64,
    #[serde(rename = "STUN_URLS")]
    pub stun_urls: String,
    #[serde(rename = "TURN_URLS")]
    pub turn_urls: String,
    #[serde(rename = "IS_SERVICE")]
    pub is_service: bool,
    /// True when this project is one half of a `role=both` generation pair.
    #[serde(rename = "IS_BOTH")]
    pub is_both: bool,
}

impl TemplateContext {
    pub fn new(
        project_name: &str,
        signaling_url: &str,
        manufacturer: &str,
        service_name: &str,
        is_service: bool,
    ) -> Self {
        let project_name_pascal = to_pascal_case(project_name);
        Self {
            project_name: project_name.to_string(),
            project_name_snake: to_snake_case(project_name),
            project_name_pascal: project_name_pascal.clone(),
            signaling_url: signaling_url.to_string(),
            manufacturer: manufacturer.to_string(),
            service_name: service_name.to_string(),
            workload_name: format!("{}Workload", project_name_pascal),
            actr_swift_version: DEFAULT_ACTR_SWIFT_VERSION.to_string(),
            actr_protocols_version: DEFAULT_ACTR_PROTOCOLS_VERSION.to_string(),
            actr_local_path: resolve_actr_swift_local_path(),
            actr_web_local_path: resolve_actr_web_local_path(),
            realm_id: 2368266035,
            stun_urls: r#"["stun:actrix1.develenv.com:3478"]"#.to_string(),
            turn_urls: r#"["turn:actrix1.develenv.com:3478"]"#.to_string(),
            is_service,
            is_both: false,
        }
    }

    pub async fn new_with_versions(
        project_name: &str,
        signaling_url: &str,
        manufacturer: &str,
        service_name: &str,
        is_service: bool,
    ) -> Self {
        let mut ctx = Self::new(
            project_name,
            signaling_url,
            manufacturer,
            service_name,
            is_service,
        );

        // Fetch latest versions in parallel with 5s timeout
        let swift_task = crate::utils::fetch_latest_git_tag(
            "https://github.com/actor-rtc/actr-swift",
            &ctx.actr_swift_version,
        );
        let protocols_task = crate::utils::fetch_latest_git_tag(
            "https://github.com/actor-rtc/actr-protocols-swift",
            &ctx.actr_protocols_version,
        );

        let (swift_v, protocols_v) = tokio::join!(swift_task, protocols_task);

        ctx.actr_swift_version = swift_v;
        ctx.actr_protocols_version = protocols_v;

        ctx
    }
}

fn resolve_actr_swift_local_path() -> Option<String> {
    let base = std::env::var("ACTR_SWIFT_LOCAL_PATH").ok()?;
    let root = Path::new(&base);
    let candidates: [PathBuf; 4] = [
        root.to_path_buf(),
        root.join("actr-swift"),
        root.join("bindings/swift"),
        root.join("actr/bindings/swift"),
    ];

    candidates
        .into_iter()
        .find(|path| path.join("Package.swift").is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

fn resolve_actr_web_local_path() -> Option<String> {
    let base = std::env::var("ACTR_WEB_LOCAL_PATH").ok()?;
    let root = Path::new(&base);
    let candidates: [PathBuf; 4] = [
        root.to_path_buf(),
        root.join("web-sdk"),
        root.join("bindings/web/packages/web-sdk"),
        root.join("actr/bindings/web/packages/web-sdk"),
    ];

    candidates
        .into_iter()
        .find(|path| path.join("package.json").is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

pub trait LangTemplate: Send + Sync {
    fn load_files(
        &self,
        template_name: ProjectTemplateName,
        context: &TemplateContext,
    ) -> Result<HashMap<String, String>>;
}

pub struct ProjectTemplate {
    name: ProjectTemplateName,
    lang_template: Box<dyn LangTemplate>,
}

impl ProjectTemplate {
    pub fn new(template_name: ProjectTemplateName, language: SupportedLanguage) -> Self {
        let lang_template: Box<dyn LangTemplate> = match language {
            SupportedLanguage::Swift => Box::new(SwiftTemplate),
            SupportedLanguage::Kotlin => Box::new(KotlinTemplate),
            SupportedLanguage::Python => Box::new(PythonTemplate),
            SupportedLanguage::Rust => Box::new(RustTemplate),
            SupportedLanguage::TypeScript => Box::new(TypeScriptTemplate),
            SupportedLanguage::Web => Box::new(self::web::WebTemplate),
        };

        Self {
            name: template_name,
            lang_template,
        }
    }

    pub fn load_file(
        fixture_path: &Path,
        files: &mut HashMap<String, String>,
        key: &str,
    ) -> Result<()> {
        let content = if fixture_path.exists() {
            std::fs::read_to_string(fixture_path)?
        } else {
            // Read from embedded fixtures when running from packaged binaries.
            let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
            let relative = fixture_path
                .strip_prefix(&fixtures_root)
                .map_err(|_| {
                    ActrCliError::Io(std::io::Error::new(
                        ErrorKind::NotFound,
                        format!("Fixture not found: {}", fixture_path.display()),
                    ))
                })?
                .to_string_lossy()
                .replace('\\', "/");
            let file = FixtureAssets::get(&relative).ok_or_else(|| {
                ActrCliError::Io(std::io::Error::new(
                    ErrorKind::NotFound,
                    format!("Embedded fixture not found: {}", relative),
                ))
            })?;
            std::str::from_utf8(file.data.as_ref())
                .map_err(|error| {
                    ActrCliError::Io(std::io::Error::new(
                        ErrorKind::InvalidData,
                        format!("Invalid UTF-8 fixture {}: {}", relative, error),
                    ))
                })?
                .to_string()
        };
        files.insert(key.to_string(), content);
        Ok(())
    }

    pub fn generate(&self, project_path: &Path, context: &TemplateContext) -> Result<()> {
        let files = self.lang_template.load_files(self.name, context)?;
        let handlebars = Handlebars::new();

        for (file_path, content) in &files {
            let rendered_path = handlebars.render_template(file_path, context)?;
            let rendered_content = handlebars.render_template(content, context)?;

            let full_path = project_path.join(&rendered_path);

            // Create parent directories if they don't exist
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            std::fs::write(full_path, rendered_content)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(ctx.actr_swift_version, DEFAULT_ACTR_SWIFT_VERSION);
        assert_eq!(ctx.actr_protocols_version, DEFAULT_ACTR_PROTOCOLS_VERSION);
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
        // Verify Actr.toml exists
        assert!(temp_dir.path().join("Actr.toml").exists());
        // Verify .gitignore exists
        assert!(temp_dir.path().join(".gitignore").exists());
        // Note: proto files are no longer created during init, they will be pulled via actr install
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
}
