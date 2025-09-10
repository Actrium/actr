//! Run command implementation

use crate::commands::{build::BuildCommand, Command};
use crate::error::{ActrCliError, Result};
use crate::utils::{execute_command_streaming, get_target_dir, is_actr_project, warn_if_not_actr_project};
use actr_config::ActrConfig;
use async_trait::async_trait;
use clap::Args;
use std::path::Path;
use tracing::{info, debug};

#[derive(Args)]
pub struct RunCommand {
    /// Run in release mode
    #[arg(long)]
    pub release: bool,

    /// Build before running (default: true)
    #[arg(long, default_value = "true")]
    pub build: bool,

    /// Arguments to pass to the running program
    #[arg(last = true)]
    pub program_args: Vec<String>,
}

#[async_trait]
impl Command for RunCommand {
    async fn execute(&self) -> Result<()> {
        info!("🚀 Running Actor-RTC project");

        // Check that we're in an Actor-RTC project
        warn_if_not_actr_project();

        let project_root = std::env::current_dir()?;

        // Load configuration if available
        let config = if is_actr_project() {
            Some(ActrConfig::from_file("actr.toml")?)
        } else {
            None
        };

        // Build first if requested
        if self.build {
            info!("Building project before running");
            let build_cmd = BuildCommand {
                release: self.release,
                skip_proto_deps: false,
                clean: false,
                cargo_args: vec![],
            };
            build_cmd.execute().await?;
        }

        // Determine the executable name
        let executable_name = self.get_executable_name(&config, &project_root)?;
        
        // Run the executable
        self.run_executable(&executable_name, &project_root).await?;

        Ok(())
    }
}

impl RunCommand {
    fn get_executable_name(&self, config: &Option<ActrConfig>, project_root: &Path) -> Result<String> {
        if let Some(config) = config {
            // Use the package name from actr.toml
            Ok(config.package.name.clone())
        } else {
            // Try to read from Cargo.toml
            let cargo_toml_path = project_root.join("Cargo.toml");
            if cargo_toml_path.exists() {
                let content = std::fs::read_to_string(cargo_toml_path)?;
                
                // Simple parsing to extract package name
                for line in content.lines() {
                    if line.trim().starts_with("name") && line.contains("=") {
                        let parts: Vec<&str> = line.split('=').collect();
                        if parts.len() == 2 {
                            let name = parts[1].trim().trim_matches('"').trim();
                            return Ok(name.to_string());
                        }
                    }
                }
            }
            
            // Fallback to directory name
            project_root
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_string())
                .ok_or_else(|| ActrCliError::InvalidProject(
                    "Cannot determine project name".to_string()
                ))
        }
    }

    async fn run_executable(&self, executable_name: &str, project_root: &Path) -> Result<()> {
        let target_dir = get_target_dir(project_root);
        
        let profile = if self.release { "release" } else { "debug" };
        let executable_path = target_dir.join(profile).join(executable_name);

        // Check if executable exists
        if !executable_path.exists() {
            return Err(ActrCliError::BuildFailed(format!(
                "Executable not found: {}. Did the build succeed?",
                executable_path.display()
            )));
        }

        info!("Running: {}", executable_path.display());

        // Prepare arguments
        let mut args = Vec::new();
        for arg in &self.program_args {
            args.push(arg.as_str());
        }

        // Set up environment variables for Actor-RTC
        std::env::set_var("RUST_LOG", std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()));

        // Run the executable
        execute_command_streaming(
            executable_path.to_str().unwrap(),
            &args,
            Some(project_root),
        ).await?;

        Ok(())
    }

    /// Run the project using cargo run (alternative implementation)
    #[allow(dead_code)]
    async fn run_with_cargo(&self, project_root: &Path) -> Result<()> {
        let mut args = vec!["run"];
        
        if self.release {
            args.push("--release");
        }

        // Add separator for program arguments
        if !self.program_args.is_empty() {
            args.push("--");
            for arg in &self.program_args {
                args.push(arg);
            }
        }

        debug!("Running cargo with args: {:?}", args);
        execute_command_streaming("cargo", &args, Some(project_root)).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_get_executable_name_from_config() {
        let config = Some(ActrConfig::default_template("my-test-service"));
        let temp_dir = TempDir::new().unwrap();
        
        let cmd = RunCommand {
            release: false,
            build: true,
            program_args: vec![],
        };

        let name = cmd.get_executable_name(&config, temp_dir.path()).unwrap();
        assert_eq!(name, "my-test-service");
    }

    #[test]
    fn test_get_executable_name_from_cargo_toml() {
        let temp_dir = TempDir::new().unwrap();
        let cargo_toml = temp_dir.path().join("Cargo.toml");
        
        std::fs::write(&cargo_toml, r#"
[package]
name = "test-project"
version = "0.1.0"
"#).unwrap();

        let cmd = RunCommand {
            release: false,
            build: true,
            program_args: vec![],
        };

        let name = cmd.get_executable_name(&None, temp_dir.path()).unwrap();
        assert_eq!(name, "test-project");
    }

    #[test]
    fn test_get_executable_name_fallback() {
        let temp_dir = TempDir::new().unwrap();
        
        let cmd = RunCommand {
            release: false,
            build: true,
            program_args: vec![],
        };

        let name = cmd.get_executable_name(&None, temp_dir.path()).unwrap();
        // Should be the temporary directory name
        assert!(!name.is_empty());
    }
}