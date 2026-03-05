use crate::commands::SupportedLanguage;
use crate::commands::initialize::traits::{InitContext, ProjectInitializer};
use crate::commands::initialize::{create_local_proto, create_protoc_plugin_config, init_git_repo};
use crate::error::Result;
use crate::template::{EchoRole, ProjectTemplate, TemplateContext};
use async_trait::async_trait;
use tracing::info;

pub struct TypeScriptInitializer;

#[async_trait]
impl ProjectInitializer for TypeScriptInitializer {
    async fn generate_project_structure(&self, context: &InitContext) -> Result<()> {
        let is_service = context.echo_role == Some(EchoRole::Service);

        let template = ProjectTemplate::new(context.template, SupportedLanguage::TypeScript);
        let mut template_context = TemplateContext::new(
            &context.project_name,
            &context.signaling_url,
            &context.manufacturer,
            context.template.to_service_name(),
            is_service,
        );
        template_context.is_both = context.is_both;

        template.generate(&context.project_dir, &template_context)?;

        create_local_proto(
            &context.project_dir,
            &context.project_name,
            "protos/local",
            context.template,
            context.echo_role,
        )?;
        create_protoc_plugin_config(&context.project_dir)?;
        init_git_repo(&context.project_dir)?;

        Ok(())
    }

    fn print_next_steps(&self, context: &InitContext) {
        info!("");
        info!("Next steps:");
        if !context.is_current_dir {
            info!("  cd {}", context.project_dir.display());
        }
        if context.echo_role == Some(EchoRole::Service) {
            info!("  actr install      # Create Actr.lock.toml and install npm dependencies");
            info!("  actr gen -l typescript  # Generate Actor framework code from local proto");
            info!("  npm run dev       # Start the EchoService (Ctrl+C to stop)");
        } else {
            info!(
                "  actr install      # Download remote proto dependencies and install npm packages"
            );
            info!("  actr gen -l typescript  # Generate Actor framework code");
            info!("  npm run dev       # Run the echo app");
        }
    }
}
