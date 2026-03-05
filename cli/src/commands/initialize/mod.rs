mod kotlin;
mod python;
mod rust;
mod swift;
pub mod traits;
mod typescript;

use crate::commands::SupportedLanguage;
use crate::error::{ActrCliError, Result};
use crate::template::{DEFAULT_MANUFACTURER, EchoRole, ProjectTemplateName, TemplateContext};
use crate::utils::read_fixture_text;
use handlebars::Handlebars;
use kotlin::KotlinInitializer;
use python::PythonInitializer;
use rust::RustInitializer;
use std::path::Path;
use std::process::Command;
use swift::SwiftInitializer;
use typescript::TypeScriptInitializer;

pub use traits::{InitContext, ProjectInitializer};

/// Create .protoc-plugin.toml with default minimum versions.
pub fn create_protoc_plugin_config(project_dir: &Path) -> Result<()> {
    const DEFAULT_PLUGIN_MIN_VERSION: &str = "0.1.10";

    let config_path = project_dir.join(".protoc-plugin.toml");
    if config_path.exists() {
        return Ok(());
    }

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = format!(
        "version = 1\n\n[plugins]\nprotoc-gen-actrframework = \"{DEFAULT_PLUGIN_MIN_VERSION}\"\nprotoc-gen-actrframework-swift = \"{DEFAULT_PLUGIN_MIN_VERSION}\"\nprotoc-gen-actrframework-typescript = \"{DEFAULT_PLUGIN_MIN_VERSION}\"\n"
    );

    std::fs::write(&config_path, content)?;
    tracing::info!("📄 Created .protoc-plugin.toml");
    Ok(())
}

pub fn init_git_repo(project_dir: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["init"])
        .current_dir(project_dir)
        .output()
        .map_err(|error| ActrCliError::Command(format!("Failed to run git init: {error}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ActrCliError::Command(format!("git init failed: {stderr}")));
    }

    tracing::info!("🔧 Initialized git repository");
    Ok(())
}

/// Generate a local.proto file with the given package name.
///
/// For the echo service role, uses the full EchoService definition so that
/// `actr gen` can produce `EchoHandler` / `EchoServiceActor` traits.
/// For the echo app role and other templates, generates an empty skeleton.
pub fn create_local_proto(
    project_dir: &Path,
    project_name: &str,
    proto_dir: &str,
    template: ProjectTemplateName,
    echo_role: Option<EchoRole>,
) -> Result<()> {
    let proto_path = project_dir.join(proto_dir);
    std::fs::create_dir_all(&proto_path)?;

    // Load template file
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let template_file_name = match (template, echo_role) {
        (ProjectTemplateName::Echo, Some(EchoRole::Service)) => "echo_service.hbs",
        (ProjectTemplateName::Echo, _) => "local.echo.hbs",
        (ProjectTemplateName::DataStream, _) => "local.data-stream.hbs",
    };
    let template_path = fixtures_root.join("protos").join(template_file_name);

    let template_content = read_fixture_text(&template_path)?;

    // Create template context
    let template_context = TemplateContext::new(project_name, "", DEFAULT_MANUFACTURER, "", false);
    let handlebars = Handlebars::new();

    // Render template
    let local_proto_content = handlebars
        .render_template(&template_content, &template_context)
        .map_err(|e| {
            ActrCliError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to render proto template: {}", e),
            ))
        })?;

    let proto_output_name = match (template, echo_role) {
        (ProjectTemplateName::Echo, Some(EchoRole::Service)) => "echo.proto",
        _ => "local.proto",
    };
    let proto_output_path = proto_path.join(proto_output_name);
    std::fs::write(&proto_output_path, local_proto_content)?;

    tracing::info!("📄 Created {}", proto_output_path.display());
    Ok(())
}

pub struct InitializerFactory;

impl InitializerFactory {
    pub fn get_initializer(language: SupportedLanguage) -> Result<Box<dyn ProjectInitializer>> {
        match language {
            SupportedLanguage::Rust => Ok(Box::new(RustInitializer)),
            SupportedLanguage::Python => Ok(Box::new(PythonInitializer)),
            SupportedLanguage::Swift => Ok(Box::new(SwiftInitializer)),
            SupportedLanguage::Kotlin => Ok(Box::new(KotlinInitializer)),
            SupportedLanguage::TypeScript => Ok(Box::new(TypeScriptInitializer)),
        }
    }
}

pub async fn execute_initialize(language: SupportedLanguage, context: &InitContext) -> Result<()> {
    let initializer = InitializerFactory::get_initializer(language)?;
    initializer.generate_project_structure(context).await?;
    initializer.print_next_steps(context);
    Ok(())
}
