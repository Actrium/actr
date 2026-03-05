use crate::template::EchoRole;
use crate::{error::Result, template::ProjectTemplateName};
use async_trait::async_trait;
use std::path::PathBuf;

/// Context for project initialization.
#[derive(Debug, Clone)]
pub struct InitContext {
    pub project_dir: PathBuf,
    pub project_name: String,
    pub signaling_url: String,
    pub manufacturer: String,
    pub template: ProjectTemplateName,
    pub is_current_dir: bool,
    /// Role for echo template: service or app. Ignored for other templates.
    pub echo_role: Option<EchoRole>,
    /// True when this project is being generated as part of a `role=both` pair.
    /// Causes the app to depend on the locally-generated echo-service rather than
    /// the public echo-echo-server registry package.
    pub is_both: bool,
}

/// Interface for language-specific project initialization.
#[async_trait]
pub trait ProjectInitializer: Send + Sync {
    async fn generate_project_structure(&self, context: &InitContext) -> Result<()>;
    fn print_next_steps(&self, context: &InitContext);
}
