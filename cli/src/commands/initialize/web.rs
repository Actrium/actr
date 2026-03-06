use crate::commands::SupportedLanguage;
use crate::commands::initialize::traits::{InitContext, ProjectInitializer};
use crate::commands::initialize::{create_local_proto, create_protoc_plugin_config, init_git_repo};
use crate::error::Result;
use crate::template::{EchoRole, ProjectTemplate, TemplateContext};
use async_trait::async_trait;
use tracing::info;

pub struct WebInitializer;

#[async_trait]
impl ProjectInitializer for WebInitializer {
    async fn generate_project_structure(&self, context: &InitContext) -> Result<()> {
        let is_service = context.echo_role == Some(EchoRole::Service);

        let template = ProjectTemplate::new(context.template, SupportedLanguage::Web);
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

        // Create public directory for Service Worker
        std::fs::create_dir_all(context.project_dir.join("public"))?;

        init_git_repo(&context.project_dir)?;

        Ok(())
    }

    fn print_next_steps(&self, context: &InitContext) {
        info!("");
        info!("Next steps:");
        if !context.is_current_dir {
            info!("  cd {}", context.project_dir.display());
        }
        info!("  npm install           # Install dependencies");
        info!("  actr install          # Create Actr.lock.toml");
        info!(
            "  actr gen -l web       # Generate code (TypeScript types, WASM scaffold, actor.sw.js)"
        );
        info!("  cd wasm && bash build.sh && cd ..  # Build WASM");
        info!("  npm run dev           # Start the development server");
    }
}
