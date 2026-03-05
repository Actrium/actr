use crate::commands::SupportedLanguage;
use crate::commands::initialize::traits::{InitContext, ProjectInitializer};
use crate::commands::initialize::{create_local_proto, create_protoc_plugin_config};
use crate::error::{ActrCliError, Result};
use crate::template::{EchoRole, ProjectTemplate, ProjectTemplateName, TemplateContext};
use crate::utils::read_fixture_text;
use async_trait::async_trait;
use handlebars::Handlebars;
use std::path::Path;
use tracing::info;

pub struct RustInitializer;

#[async_trait]
impl ProjectInitializer for RustInitializer {
    async fn generate_project_structure(&self, context: &InitContext) -> Result<()> {
        info!("⚡ Generating Rust project structure...");

        let is_service = context.echo_role == Some(EchoRole::Service);

        // 1. Initialize with templates
        let template = ProjectTemplate::new(context.template, SupportedLanguage::Rust);
        let mut template_context = TemplateContext::new(
            &context.project_name,
            &context.signaling_url,
            &context.manufacturer,
            context.template.to_service_name(),
            is_service,
        );
        template_context.is_both = context.is_both;

        template.generate(&context.project_dir, &template_context)?;

        // 2. Create local proto.
        if context.template == ProjectTemplateName::Echo && !is_service {
            self.create_bridge_proto(context)?;
        } else {
            create_local_proto(
                &context.project_dir,
                &context.project_name,
                "protos/local",
                context.template,
                context.echo_role,
            )?;
        }

        // 3. Create .protoc-plugin.toml
        create_protoc_plugin_config(&context.project_dir)?;

        Ok(())
    }

    fn print_next_steps(&self, context: &InitContext) {
        println!("\nNext steps:");
        if !context.is_current_dir {
            println!("  cd {}", context.project_dir.display());
        }
        if context.echo_role == Some(EchoRole::Service) {
            println!(
                "  actr install      # Create Actr.lock.toml (no remote deps, generates empty lock)"
            );
            println!("  actr gen -l rust  # Generate Actor framework code from local proto");
            println!("  cargo run         # Start the EchoService (Ctrl+C to stop)");
        } else {
            println!("  actr install      # Download remote proto dependencies from registry");
            println!("  actr gen -l rust  # Generate Actor framework code");
            println!("  cargo run         # Run the echo app");
        }
    }
}

impl RustInitializer {
    fn create_bridge_proto(&self, context: &InitContext) -> Result<()> {
        let proto_dir = context.project_dir.join("protos/local");
        std::fs::create_dir_all(&proto_dir)?;

        let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        let template_path = fixtures_root.join("protos/local.echo.rust-client.hbs");
        let template = read_fixture_text(&template_path)?;
        let template_context =
            TemplateContext::new(&context.project_name, "", &context.manufacturer, "", false);

        let content = Handlebars::new()
            .render_template(&template, &template_context)
            .map_err(|e| {
                ActrCliError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Failed to render proto template: {e}"),
                ))
            })?;

        let output_path = proto_dir.join("local.proto");
        std::fs::write(&output_path, content)?;
        info!("📄 Created {}", output_path.display());
        Ok(())
    }
}
