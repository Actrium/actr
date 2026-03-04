//! Project initialization command

use crate::commands::Command;
use crate::error::{ActrCliError, Result};
use async_trait::async_trait;
use clap::Args;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Args)]
pub struct InitCommand {
    /// Name of the project to create (use '.' for current directory)
    pub name: Option<String>,

    /// Project template to use (echo, chat-room, etc.)
    #[arg(long)]
    pub template: Option<String>,

    /// Project name when initializing in current directory
    #[arg(long)]
    pub project_name: Option<String>,

    /// Signaling server URL
    #[arg(long)]
    pub signaling: Option<String>,
}

#[async_trait]
impl Command for InitCommand {
    async fn execute(&self) -> Result<()> {
        // Show welcome header
        println!("🎯 Actor-RTC Project Initialization");
        println!("----------------------------------------");

        // Interactive prompt for missing required fields
        let name = self.prompt_if_missing("project name", self.name.as_ref())?;
        let signaling_url =
            self.prompt_if_missing("signaling server URL", self.signaling.as_ref())?;

        let (project_dir, project_name) = self.resolve_project_info(&name)?;

        info!("🚀 Initializing Actor-RTC project: {}", project_name);

        // Check if target directory exists and is not empty
        if project_dir.exists() && project_dir != Path::new(".") {
            return Err(ActrCliError::InvalidProject(format!(
                "Directory '{}' already exists. Use a different name or remove the existing directory.",
                project_dir.display()
            )));
        }

        // Check if current directory already has Actr.toml
        if project_dir == Path::new(".") && Path::new("Actr.toml").exists() {
            return Err(ActrCliError::InvalidProject(
                "Current directory already contains an Actor-RTC project (Actr.toml exists)"
                    .to_string(),
            ));
        }

        // Create project directory if needed
        if project_dir != Path::new(".") {
            std::fs::create_dir_all(&project_dir)?;
        }

        // Generate project structure
        self.generate_project_structure(&project_dir, &project_name, &signaling_url)?;

        info!(
            "✅ Successfully created Actor-RTC project '{}'",
            project_name
        );
        if project_dir != Path::new(".") {
            info!("📁 Project created in: {}", project_dir.display());
            info!("");
            info!("Next steps:");
            info!("  cd {}", project_dir.display());
            info!("  actr install actr://{{some-service}}/  # Add service dependencies");
            info!("  actr gen                             # Generate Actor code");
            info!("  cargo run                            # Start your work");
        } else {
            info!("📁 Project initialized in current directory");
            info!("");
            info!("Next steps:");
            info!("  actr install actr://{{some-service}}/  # Add service dependencies");
            info!("  actr gen                             # Generate Actor code");
            info!("  cargo run                            # Start your work");
        }

        Ok(())
    }
}

impl InitCommand {
    fn resolve_project_info(&self, name: &str) -> Result<(PathBuf, String)> {
        if name == "." {
            // Initialize in current directory - cargo will determine the name
            let project_name = if let Some(name) = &self.project_name {
                name.clone()
            } else {
                // Let cargo determine the project name from directory
                "current-dir".to_string() // Placeholder - cargo will override
            };
            Ok((PathBuf::from("."), project_name))
        } else {
            // Create new directory
            Ok((PathBuf::from(name), name.to_string()))
        }
    }

    fn generate_project_structure(
        &self,
        project_dir: &Path,
        project_name: &str,
        signaling_url: &str,
    ) -> Result<()> {
        // Always use cargo init for all scenarios
        if project_dir == Path::new(".") {
            // Current directory init - let cargo handle naming
            self.init_with_cargo(project_dir, None, signaling_url)?;
        } else {
            // New directory - create it and use cargo init with explicit name
            std::fs::create_dir_all(project_dir)?;
            self.init_with_cargo(project_dir, Some(project_name), signaling_url)?;
        }

        Ok(())
    }

    fn create_actr_config(
        &self,
        project_dir: &Path,
        project_name: &str,
        signaling_url: &str,
    ) -> Result<()> {
        let _template_name = self.template.as_deref().unwrap_or("minimal");
        let service_type = format!("{project_name}-service");

        // Create Actr.toml directly as string (Config doesn't have default_template or save_to_file)
        let actr_toml_content = format!(
            r#"edition = 1
exports = []

[package]
name = "{project_name}"
description = "An Actor-RTC service"
authors = []

[package.actr_type]
manufacturer = "my-company"
name = "{service_type}"
version = "0.1.0"

[dependencies]

[system.signaling]
url = "{signaling_url}"

[system.deployment]
realm_id = 1001

[system.discovery]
visible = true

[scripts]
dev = "cargo run"
test = "cargo test"
"#
        );

        std::fs::write(project_dir.join("Actr.toml"), actr_toml_content)?;

        info!("📄 Created Actr.toml configuration");
        Ok(())
    }

    fn create_gitignore(&self, project_dir: &Path) -> Result<()> {
        let gitignore_content = r#"/target
/Cargo.lock
.env
.env.local
*.log
.DS_Store
/src/generated/
"#;

        std::fs::write(project_dir.join(".gitignore"), gitignore_content)?;

        info!("📄 Created .gitignore");
        Ok(())
    }

    /// Interactive prompt for missing fields with detailed guidance
    fn prompt_if_missing(
        &self,
        field_name: &str,
        current_value: Option<&String>,
    ) -> Result<String> {
        if let Some(value) = current_value {
            return Ok(value.clone());
        }

        match field_name {
            "project name" => {
                println!("┌──────────────────────────────────────────────────────────┐");
                println!("│ 📋  Project Name Configuration                           │");
                println!("├──────────────────────────────────────────────────────────┤");
                println!("│                                                          │");
                println!("│  📝 Requirements:                                        │");
                println!("│     • Only alphanumeric characters, hyphens and _        │");
                println!("│     • Cannot start or end with - or _                    │");
                println!("│                                                          │");
                println!("│  💡 Examples:                                            │");
                println!("│     my-chat-service, user-manager, media_streamer        │");
                println!("│                                                          │");
                println!("└──────────────────────────────────────────────────────────┘");
                print!("🎯 Enter project name [my-actor-project]: ");
            }
            "signaling server URL" => {
                println!("┌──────────────────────────────────────────────────────────┐");
                println!("│ 🌐  Signaling Server Configuration                       │");
                println!("├──────────────────────────────────────────────────────────┤");
                println!("│                                                          │");
                println!("│  📡 WebSocket URL for Actor-RTC signaling coordination   │");
                println!("│                                                          │");
                println!("│  💡 Examples:                                            │");
                println!("│     ws://localhost:8080/                (development)    │");
                println!("│     wss://example.com                   (production      │");
                println!("│     wss://example.com/?token=${{TOKEN}}   (with auth)      │");
                println!("│                                                          │");
                println!("└──────────────────────────────────────────────────────────┘");
                print!("🎯 Enter signaling server URL [ws://localhost:8080/]: ");
            }
            _ => {
                print!("🎯 Enter {field_name}: ");
            }
        }

        io::stdout().flush().map_err(ActrCliError::Io)?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(ActrCliError::Io)?;

        println!();

        let trimmed = input.trim();
        if trimmed.is_empty() {
            // Provide sensible defaults
            let default = match field_name {
                "project name" => "my-actor-project",
                "signaling server URL" => "ws://localhost:8080/",
                _ => {
                    return Err(ActrCliError::InvalidProject(format!(
                        "{field_name} cannot be empty"
                    )));
                }
            };
            Ok(default.to_string())
        } else {
            // Validate project name if applicable
            if field_name == "project name" {
                self.validate_project_name(trimmed)?;
            }
            Ok(trimmed.to_string())
        }
    }

    /// Validate project name according to requirements
    fn validate_project_name(&self, name: &str) -> Result<()> {
        // Check if name is valid: alphanumeric characters, hyphens, and underscores only
        let is_valid = name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');

        if !is_valid {
            return Err(ActrCliError::InvalidProject(format!(
                "Invalid project name '{name}'. Only alphanumeric characters, hyphens, and underscores are allowed."
            )));
        }

        // Check for other common invalid patterns
        if name.is_empty() {
            return Err(ActrCliError::InvalidProject(
                "Project name cannot be empty".to_string(),
            ));
        }

        if name.starts_with('-') || name.ends_with('-') {
            return Err(ActrCliError::InvalidProject(
                "Project name cannot start or end with a hyphen".to_string(),
            ));
        }

        if name.starts_with('_') || name.ends_with('_') {
            return Err(ActrCliError::InvalidProject(
                "Project name cannot start or end with an underscore".to_string(),
            ));
        }

        Ok(())
    }

    /// Initialize using cargo init, then enhance for Actor-RTC
    fn init_with_cargo(
        &self,
        project_dir: &Path,
        explicit_name: Option<&str>,
        signaling_url: &str,
    ) -> Result<()> {
        info!("🚀 Initializing Rust project with cargo...");

        // Step 1: Run cargo init - let it handle all validation
        let mut cmd = std::process::Command::new("cargo");
        cmd.arg("init").arg("--quiet").current_dir(project_dir);

        // Add explicit name if provided (for new directories)
        if let Some(name) = explicit_name {
            cmd.arg("--name").arg(name);
        }

        let cargo_result = cmd
            .output()
            .map_err(|e| ActrCliError::Command(format!("Failed to run cargo init: {e}")))?;

        if !cargo_result.status.success() {
            let error_msg = String::from_utf8_lossy(&cargo_result.stderr);
            return Err(ActrCliError::Command(format!(
                "cargo init failed: {error_msg}"
            )));
        }

        // Step 2: Read the project name that cargo determined
        let project_name = self.extract_project_name_from_cargo_toml(project_dir)?;
        info!("📦 Rust project initialized: '{}'", project_name);

        // Step 3: Enhance with Actor-RTC specific files
        self.enhance_cargo_project_for_actr(project_dir, &project_name, signaling_url)?;

        Ok(())
    }

    /// Extract project name from Cargo.toml generated by cargo init
    fn extract_project_name_from_cargo_toml(&self, project_dir: &Path) -> Result<String> {
        let cargo_toml_path = project_dir.join("Cargo.toml");
        let cargo_content = std::fs::read_to_string(&cargo_toml_path).map_err(ActrCliError::Io)?;

        // Parse TOML to extract project name
        for line in cargo_content.lines() {
            if line.trim().starts_with("name = ") {
                if let Some(name_part) = line.split('=').nth(1) {
                    let name = name_part.trim().trim_matches('"').trim_matches('\'');
                    return Ok(name.to_string());
                }
            }
        }

        // Fallback to directory name if parsing fails
        Ok("actor-service".to_string())
    }

    /// Enhance cargo-generated project with Actor-RTC specific features
    fn enhance_cargo_project_for_actr(
        &self,
        project_dir: &Path,
        project_name: &str,
        signaling_url: &str,
    ) -> Result<()> {
        info!("⚡ Enhancing with Actor-RTC features...");

        // Create proto directory
        let proto_dir = project_dir.join("proto");
        std::fs::create_dir_all(&proto_dir)?;
        info!("📁 Created proto/ directory");

        // Generate Actr.toml
        self.create_actr_config(project_dir, project_name, signaling_url)?;
        info!("📄 Created Actr.toml configuration");

        // Enhance Cargo.toml with Actor-RTC dependencies
        self.enhance_cargo_toml_with_actr_deps(project_dir)?;
        info!("📦 Enhanced Cargo.toml with Actor-RTC dependencies");

        // Create .gitignore if it doesn't exist
        let gitignore_path = project_dir.join(".gitignore");
        if !gitignore_path.exists() {
            self.create_gitignore(project_dir)?;
            info!("📄 Created .gitignore");
        }

        Ok(())
    }

    /// Add Actor-RTC dependencies to existing Cargo.toml
    fn enhance_cargo_toml_with_actr_deps(&self, project_dir: &Path) -> Result<()> {
        let cargo_toml_path = project_dir.join("Cargo.toml");
        let mut cargo_content =
            std::fs::read_to_string(&cargo_toml_path).map_err(ActrCliError::Io)?;

        // Add Actor-RTC dependencies if not already present
        if !cargo_content.contains("actr-core") {
            cargo_content.push_str("\n# Actor-RTC Framework Dependencies\n");
            cargo_content.push_str("actr-core = { path = \"../actr-core\" }\n");
            cargo_content.push_str("tokio = { version = \"1.0\", features = [\"full\"] }\n");

            std::fs::write(&cargo_toml_path, cargo_content).map_err(ActrCliError::Io)?;
        }

        Ok(())
    }
}
