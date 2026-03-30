use super::{InitContext, ProjectInitializer, create_protoc_plugin_config};

use crate::commands::SupportedLanguage;
use crate::error::Result;
use crate::template::{ProjectTemplate, TemplateContext};
use async_trait::async_trait;
use tracing::info;

pub struct PythonInitializer;

#[async_trait]
impl ProjectInitializer for PythonInitializer {
    async fn generate_project_structure(&self, context: &InitContext) -> Result<()> {
        let template = ProjectTemplate::new(context.template, SupportedLanguage::Python);
        let service_name = context.template.to_service_name();
        let template_context = TemplateContext::new(
            &context.project_name,
            &context.signaling_url,
            &context.manufacturer,
            service_name,
            false,
        );
        template.generate(&context.project_dir, &template_context)?;
        create_protoc_plugin_config(&context.project_dir.join("server"))?;
        create_protoc_plugin_config(&context.project_dir.join("client"))?;

        // Note: Proto files are now created via templates:
        //   - server/protos/local/echo.proto (service definition)
        //   - client/protos/local/.gitkeep (empty directory placeholder)
        // We don't auto-run 'actr gen' here because it requires manifest.lock.toml
        // Users should run 'actr install' first, then 'actr gen'

        Ok(())
    }

    fn print_next_steps(&self, context: &InitContext) {
        info!("");
        info!("Next steps:");
        if !context.is_current_dir {
            info!("  cd {}", context.project_dir.display());
        }
        info!("  cd server");
        info!(
            "  #First update manifest.toml with your signaling URL, TURN/STUN server, and realm ID"
        );
        info!("  actr install  # Install remote protobuf dependencies from manifest.toml");
        info!("  actr gen -l python -i protos -o generated  # Generate code for server");
        info!("  python server.py --manifest-toml manifest.toml");
        info!("  cd ../client");
        info!(
            "  #First update manifest.toml with your signaling URL, TURN/STUN server, and realm ID"
        );
        info!("  actr install  # Install remote protobuf dependencies from manifest.toml");
        info!("  actr gen -l python -i protos -o generated  # Generate code for client");
        info!("  python client.py --manifest-toml manifest.toml");
    }
}
