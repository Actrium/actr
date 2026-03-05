//! Run command implementation

use crate::commands::Command;
use crate::error::{ActrCliError, Result};
use crate::utils::{execute_command_streaming, is_actr_project, warn_if_not_actr_project};
use actr_config::ConfigParser;
use async_trait::async_trait;
use clap::Args;
use tracing::info;

#[derive(Args)]
pub struct RunCommand {
    /// Script name to run (defaults to "run")
    pub script_name: Option<String>,
}

#[async_trait]
impl Command for RunCommand {
    async fn execute(&self) -> Result<()> {
        info!("🚀 Running Actor-RTC project");

        // Check that we're in an Actor-RTC project
        warn_if_not_actr_project();

        let _project_root = std::env::current_dir()?;

        // Load configuration if available
        let config = if is_actr_project() {
            Some(ConfigParser::from_file("Actr.toml")?)
        } else {
            None
        };

        // Get script command from configuration
        let script_name = self.script_name.as_deref().unwrap_or("run");

        let Some(ref config) = config else {
            return Err(ActrCliError::command_error(
                "No Actr.toml found. Run 'actr init' to create a project.".to_string(),
            ));
        };

        let available_scripts = config.list_scripts();

        let Some(command) = config.get_script(script_name) else {
            if available_scripts.is_empty() {
                return Err(ActrCliError::command_error(
                    "No scripts defined in Actr.toml. Add a [scripts] section.".to_string(),
                ));
            }
            return Err(ActrCliError::command_error(format!(
                "Script '{}' not found. Available scripts: {}",
                script_name,
                available_scripts.join(", ")
            )));
        };

        info!("📜 Executing script '{}': {}", script_name, command);

        // Execute the script command
        self.run_script_command(command).await?;

        Ok(())
    }
}

impl RunCommand {
    async fn run_script_command(&self, command: &str) -> Result<()> {
        // Parse command into program and args
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Err(ActrCliError::command_error("Empty command".to_string()));
        }

        let program = parts[0];
        let args = parts[1..].to_vec();

        info!("▶️  Executing: {} {}", program, args.join(" "));

        // Execute the command
        execute_command_streaming(program, &args, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_name_default() {
        let cmd = RunCommand { script_name: None };

        let script_name = cmd.script_name.as_deref().unwrap_or("run");
        assert_eq!(script_name, "run");
    }

    #[test]
    fn test_script_name_custom() {
        let cmd = RunCommand {
            script_name: Some("test".to_string()),
        };

        let script_name = cmd.script_name.as_deref().unwrap_or("run");
        assert_eq!(script_name, "test");
    }
}
