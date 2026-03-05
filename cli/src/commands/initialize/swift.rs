use super::{
    InitContext, ProjectInitializer, create_local_proto, create_protoc_plugin_config, init_git_repo,
};
use crate::commands::SupportedLanguage;
use crate::error::{ActrCliError, Result};
use crate::template::{ProjectTemplate, TemplateContext};
use async_trait::async_trait;
use std::path::Path;
use std::process::Command;
use tracing::info;

pub struct SwiftInitializer;

#[async_trait]
impl ProjectInitializer for SwiftInitializer {
    async fn generate_project_structure(&self, context: &InitContext) -> Result<()> {
        let template = ProjectTemplate::new(context.template, SupportedLanguage::Swift);
        // For data-stream template, use "LocalFileService" as the service name
        // For other templates, use the template's service name
        let service_name = match context.template {
            crate::template::ProjectTemplateName::DataStream => "LocalFileService",
            _ => context.template.to_service_name(),
        };

        // Fetch versions asynchronously (Swift echo does not use echo_role yet)
        let template_context = TemplateContext::new_with_versions(
            &context.project_name,
            &context.signaling_url,
            &context.manufacturer,
            service_name,
            false,
        )
        .await;

        // template.generate() writes all scaffold files including an empty Actr.lock.toml.
        // The lock file must exist before xcodegen runs because project.yml references it
        // as a resource; xcodegen validates file existence upfront and will fail otherwise.
        // Workflow: actr init (empty lock) -> actr install (populate) -> actr gen (read).
        template.generate(&context.project_dir, &template_context)?;
        create_protoc_plugin_config(&context.project_dir)?;

        // Create local.proto file
        create_local_proto(
            &context.project_dir,
            &context.project_name,
            "protos/local",
            context.template,
            None,
        )?;

        ensure_xcodegen_available()?;
        run_xcodegen_generate(&context.project_dir)?;

        // Create Swift Package Manager registry configuration
        create_swiftpm_registry_config(&context.project_dir)?;

        // Initialize git repository
        init_git_repo(&context.project_dir)?;

        Ok(())
    }

    fn print_next_steps(&self, context: &InitContext) {
        let template_context = TemplateContext::new(
            &context.project_name,
            &context.signaling_url,
            &context.manufacturer,
            context.template.to_service_name(),
            false,
        );
        info!("");
        info!("Next steps:");
        if !context.is_current_dir {
            info!("  cd {}", context.project_dir.display());
        }
        info!("  actr install  # Install remote protobuf dependencies from Actr.toml");
        info!(
            "  actr gen -l swift  # Use default input (protos) and Swift output ({}/Generated)",
            template_context.project_name_pascal
        );
        info!("  xcodegen generate");
        info!("  open {}.xcodeproj", template_context.project_name_pascal);
        info!("  # If you update project.yml, rerun: xcodegen generate");
    }
}

fn ensure_xcodegen_available() -> Result<()> {
    match Command::new("xcodegen").arg("--version").output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(ActrCliError::Command(format!(
                "xcodegen is not available. Install via `brew install xcodegen`. {stderr}"
            )))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(ActrCliError::Command(
            "xcodegen not found. Install via `brew install xcodegen`.".to_string(),
        )),
        Err(error) => Err(ActrCliError::Command(format!(
            "Failed to run xcodegen: {error}"
        ))),
    }
}

fn run_xcodegen_generate(project_dir: &Path) -> Result<()> {
    let output = Command::new("xcodegen")
        .arg("generate")
        .current_dir(project_dir)
        .output()
        .map_err(|error| ActrCliError::Command(format!("Failed to run xcodegen: {error}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ActrCliError::Command(format!(
            "xcodegen generate failed: {stderr}"
        )));
    }

    Ok(())
}

fn create_swiftpm_registry_config(project_dir: &Path) -> Result<()> {
    let registries_json = r#"{
    "registries": [
        {
            "url": "https://tuist.dev/api/registry/swift",
            "scopes": [
                "apple"
            ]
        }
    ]
}
"#;

    let config_path = project_dir
        .join(".swiftpm")
        .join("configuration")
        .join("registries.json");

    // Create parent directories if they don't exist
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            ActrCliError::Command(format!("Failed to create directories: {error}"))
        })?;
    }

    std::fs::write(&config_path, registries_json).map_err(|error| {
        ActrCliError::Command(format!("Failed to write registries.json: {error}"))
    })?;

    info!("📦 Created Swift Package Manager registry configuration");
    Ok(())
}
