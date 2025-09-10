//! Project initialization command

use crate::commands::Command;
use crate::error::{ActrCliError, Result};
use crate::templates::{ProjectTemplate, TemplateContext};
use actr_config::ActrConfig;
use async_trait::async_trait;
use clap::Args;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

#[derive(Args)]
pub struct InitCommand {
    /// Name of the project to create
    pub name: String,

    /// Template to use for initialization
    #[arg(long, default_value = "basic")]
    pub template: String,

    /// Directory to create the project in (defaults to current directory)
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Force creation even if directory exists
    #[arg(long)]
    pub force: bool,
}

#[async_trait]
impl Command for InitCommand {
    async fn execute(&self) -> Result<()> {
        info!("🚀 Initializing new Actor-RTC project: {}", self.name);

        // Determine the project path
        let project_path = self.get_project_path()?;

        // Check if project already exists
        if project_path.exists() && !self.force {
            if self.has_actr_files(&project_path)? {
                return Err(ActrCliError::ProjectExists(format!(
                    "Directory '{}' already contains an Actor-RTC project",
                    project_path.display()
                )));
            } else if self.directory_not_empty(&project_path)? {
                warn!("Directory '{}' is not empty but doesn't appear to be an Actor-RTC project", project_path.display());
                if !self.force {
                    return Err(ActrCliError::ProjectExists(
                        "Use --force to initialize in a non-empty directory".to_string(),
                    ));
                }
            }
        }

        // Create the project directory
        std::fs::create_dir_all(&project_path)?;

        // Generate the project from template
        let template = ProjectTemplate::load(&self.template)?;
        let context = TemplateContext::new(&self.name);
        
        template.generate(&project_path, &context)?;

        // Create the actr.toml configuration file
        self.create_actr_config(&project_path)?;

        info!("✅ Successfully created Actor-RTC project '{}'", self.name);
        info!("📁 Project created in: {}", project_path.display());
        info!("");
        info!("Next steps:");
        info!("  cd {}", project_path.display());
        info!("  actr-cli build");
        info!("  actr-cli run");

        Ok(())
    }
}

impl InitCommand {
    fn get_project_path(&self) -> Result<PathBuf> {
        let base_path = self.path.as_ref()
            .map(|p| p.clone())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        
        Ok(base_path.join(&self.name))
    }

    fn has_actr_files(&self, path: &Path) -> Result<bool> {
        Ok(path.join("actr.toml").exists() || path.join("Cargo.toml").exists())
    }

    fn directory_not_empty(&self, path: &Path) -> Result<bool> {
        if !path.exists() {
            return Ok(false);
        }

        let entries: std::result::Result<Vec<_>, _> = std::fs::read_dir(path)?.collect();
        let entries = entries?;
        Ok(!entries.is_empty())
    }

    fn create_actr_config(&self, project_path: &Path) -> Result<()> {
        let config = ActrConfig::default_template(&self.name);
        let config_path = project_path.join("actr.toml");
        config.save_to_file(config_path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_init_command_basic() {
        let temp_dir = TempDir::new().unwrap();
        let project_name = "test-project";

        let cmd = InitCommand {
            name: project_name.to_string(),
            template: "basic".to_string(),
            path: Some(temp_dir.path().to_path_buf()),
            force: false,
        };

        let result = cmd.execute().await;
        assert!(result.is_ok(), "Init command should succeed: {:?}", result);

        let project_path = temp_dir.path().join(project_name);
        assert!(project_path.exists(), "Project directory should be created");
        assert!(project_path.join("actr.toml").exists(), "actr.toml should be created");
    }

    #[test]
    fn test_project_path_resolution() {
        let cmd = InitCommand {
            name: "my-project".to_string(),
            template: "basic".to_string(),
            path: Some(PathBuf::from("/tmp")),
            force: false,
        };

        let project_path = cmd.get_project_path().unwrap();
        assert_eq!(project_path, PathBuf::from("/tmp/my-project"));
    }
}