use super::{InitContext, ProjectInitializer, init_git_repo};

use crate::commands::SupportedLanguage;
use crate::error::{ActrCliError, Result};
use crate::template::EchoRole;
use crate::template::{ProjectTemplate, TemplateContext};
use async_trait::async_trait;
use tracing::info;

pub struct PythonInitializer;

#[async_trait]
impl ProjectInitializer for PythonInitializer {
    async fn generate_project_structure(&self, context: &InitContext) -> Result<()> {
        if context.echo_role != Some(EchoRole::Service) {
            return Err(ActrCliError::Unsupported(
                "Python init now generates workload components only; use --role service."
                    .to_string(),
            ));
        }

        let template = ProjectTemplate::new(context.template, SupportedLanguage::Python);
        let service_name = context.template.to_service_name();
        let mut template_context = TemplateContext::new(
            &context.project_name,
            &context.signaling_url,
            &context.manufacturer,
            service_name,
            true,
        );
        template_context.is_both = context.is_both;
        template.generate(&context.project_dir, &template_context)?;
        make_build_script_executable(&context.project_dir)?;
        init_git_repo(&context.project_dir)?;

        Ok(())
    }

    fn print_next_steps(&self, context: &InitContext) {
        info!("");
        info!("Next steps:");
        if !context.is_current_dir {
            info!("  cd {}", context.project_dir.display());
        }
        info!("  actr deps install      # Create manifest.lock.toml");
        info!("  actr gen -l python     # Generate typed workload dispatcher and scaffold");
        info!("  ./build.sh package     # Componentize and package the workload");
    }
}

#[cfg(unix)]
fn make_build_script_executable(project_dir: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let path = project_dir.join("build.sh");
    let mut permissions = std::fs::metadata(&path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_build_script_executable(_project_dir: &std::path::Path) -> Result<()> {
    Ok(())
}
