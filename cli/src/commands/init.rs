//! Project initialization command

use crate::commands::SupportedLanguage;
use crate::commands::initialize::{self, InitContext};
use crate::config::resolver::resolve_effective_cli_config;
use crate::core::{Command, CommandContext, CommandResult, ComponentType};
use crate::error::{ActrCliError, Result};
use crate::template::{DEFAULT_MANUFACTURER, EchoRole, ProjectTemplateName};
use async_trait::async_trait;
use clap::Args;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Args)]
pub struct InitCommand {
    /// Name of the project to create (use '.' for current directory)
    pub name: Option<String>,

    /// Project template to use (echo, data-stream)
    #[arg(long, default_value_t = ProjectTemplateName::Echo)]
    pub template: ProjectTemplateName,

    /// Project name when initializing in current directory
    #[arg(long)]
    pub project_name: Option<String>,

    /// Signaling server URL
    /// TODO: will be removed when manifest.toml strips system fields
    #[arg(long)]
    pub signaling: Option<String>,

    /// Target language for project initialization
    #[arg(short, long, default_value = "rust")]
    pub language: SupportedLanguage,

    /// Role for echo template: service (provides EchoService), app (calls EchoService),
    /// or both (generate app and service projects)
    #[arg(long)]
    pub role: Option<EchoRole>,

    /// Manufacturer for generated actor types (overrides CLI config default: acme)
    #[arg(long)]
    manufacturer: Option<String>,
}

#[async_trait]
impl Command for InitCommand {
    async fn execute(&self, _ctx: &CommandContext) -> anyhow::Result<CommandResult> {
        self.execute_inner().await.map_err(anyhow::Error::from)?;
        Ok(CommandResult::Success("Project initialized".to_string()))
    }

    fn required_components(&self) -> Vec<ComponentType> {
        vec![]
    }

    fn name(&self) -> &str {
        "init"
    }

    fn description(&self) -> &str {
        "Initialize a new Actor project"
    }
}

impl InitCommand {
    async fn execute_inner(&self) -> Result<()> {
        // Resolve effective CLI config to use as defaults
        let cli_config = resolve_effective_cli_config().unwrap_or_default();

        // Show welcome header
        println!("🎯 Actor-RTC Project Initialization");
        println!("----------------------------------------");

        // Interactive prompt for missing required fields
        let name = self.prompt_if_missing("project name", self.name.as_ref())?;
        let signaling_url =
            self.prompt_if_missing("signaling server URL", self.signaling.as_ref())?;

        let echo_role = if self.template == ProjectTemplateName::Echo {
            Some(self.prompt_echo_role(self.role.as_ref())?)
        } else {
            None
        };

        // Resolve effective manufacturer from CLI args and config
        let manufacturer_owned = self.effective_manufacturer(&cli_config)?;
        let manufacturer = manufacturer_owned.as_str();

        // role=both requires custom manufacturer to avoid conflicts with public 'acme' services
        if matches!(echo_role, Some(EchoRole::Both)) && manufacturer == DEFAULT_MANUFACTURER {
            return Err(ActrCliError::InvalidProject(
                "role=both requires a custom manufacturer to avoid conflicts with public 'acme' services.\n\
                 Use: --manufacturer <your-org-name>".to_string(),
            ));
        }

        // role=service with default manufacturer will register as 'acme:EchoService', which
        // conflicts with the public echo service on the same signaling server.
        if matches!(echo_role, Some(EchoRole::Service)) && manufacturer == DEFAULT_MANUFACTURER {
            println!(
                "⚠️  Warning: using default manufacturer 'acme' with role=service will register\n\
                 this service as 'acme:EchoService', which conflicts with the public echo service\n\
                 on the same signaling server and may cause interference.\n\
                 Consider using a custom manufacturer: --manufacturer <your-org-name>"
            );
        }

        // When role=both, generate both echo-app and echo-service projects
        if matches!(echo_role, Some(EchoRole::Both)) {
            self.execute_both(&name, &signaling_url, manufacturer)
                .await?;
            return Ok(());
        }

        let (project_dir, project_name) = self.resolve_project_info(&name)?;

        info!("🚀 Initializing Actor-RTC project: {}", project_name);

        // Check if target directory exists and is not empty
        if project_dir.exists() && project_dir != Path::new(".") {
            return Err(ActrCliError::InvalidProject(format!(
                "Directory '{}' already exists. Use a different name or remove the existing directory.",
                project_dir.display()
            )));
        }

        // Check if current directory already has manifest.toml
        if project_dir == Path::new(".") && Path::new("manifest.toml").exists() {
            return Err(ActrCliError::InvalidProject(
                "Current directory already contains an ACTR workload project (manifest.toml exists)"
                    .to_string(),
            ));
        }

        // Create project directory if needed
        if project_dir != Path::new(".") {
            std::fs::create_dir_all(&project_dir)?;
        }

        // Normalize the signaling URL: strip trailing "/signaling/ws" (and optional "/")
        // so that each language template can append its own path suffix without duplication.
        let normalized_signaling_url = signaling_url
            .strip_suffix("/signaling/ws/")
            .or_else(|| signaling_url.strip_suffix("/signaling/ws"))
            .unwrap_or(&signaling_url[..])
            .trim_end_matches('/')
            .to_string();

        let context = InitContext {
            project_dir: project_dir.clone(),
            project_name: project_name.clone(),
            signaling_url: normalized_signaling_url,
            template: self.template,
            is_current_dir: project_dir == Path::new("."),
            echo_role,
            manufacturer: manufacturer.to_string(),
            is_both: false,
        };

        initialize::execute_initialize(self.language, &context).await?;

        Ok(())
    }
}

impl InitCommand {
    /// Resolve the effective manufacturer, applying precedence:
    /// CLI flag > CLI config default. Ensures the result is non-empty.
    pub fn effective_manufacturer(
        &self,
        cli_config: &crate::config::resolver::EffectiveCliConfig,
    ) -> Result<String> {
        let effective_manufacturer = cli_config.mfr.manufacturer.clone();

        let manufacturer_owned: String = match &self.manufacturer {
            Some(m) => m.clone(),
            None => effective_manufacturer,
        };

        let manufacturer = manufacturer_owned.trim();
        if manufacturer.is_empty() {
            return Err(ActrCliError::InvalidProject(
                "Manufacturer cannot be empty".to_string(),
            ));
        }

        Ok(manufacturer.to_string())
    }

    fn resolve_project_info(&self, name: &str) -> Result<(PathBuf, String)> {
        if name == "." {
            // Initialize in current directory - name will be inferred
            let project_name = if let Some(name) = &self.project_name {
                name.clone()
            } else {
                let current_dir = std::env::current_dir().map_err(|e| {
                    ActrCliError::InvalidProject(format!(
                        "Failed to resolve current directory: {e}"
                    ))
                })?;
                current_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| {
                        ActrCliError::InvalidProject(
                            "Failed to infer project name from current directory".to_string(),
                        )
                    })?
            };
            Ok((PathBuf::from("."), project_name))
        } else {
            // Create new directory - extract project name from path
            let path = PathBuf::from(name);
            let project_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(name)
                .to_string();
            Ok((path, project_name))
        }
    }

    /// Prompt for echo template role when not specified. Returns the role (never prompts if --role was given).
    fn prompt_echo_role(&self, current_value: Option<&EchoRole>) -> Result<EchoRole> {
        if let Some(role) = current_value {
            return Ok(*role);
        }

        println!("┌──────────────────────────────────────────────────────────┐");
        println!("│ 🎭  Echo Template Role                                   │");
        println!("├──────────────────────────────────────────────────────────┤");
        println!("│                                                          │");
        println!("│  service  Provides EchoService, waits for RPC calls      │");
        println!("│  app      Calls EchoService, sends echo RPC and exits    │");
        println!("│  both     Generates both app and service projects        │");
        println!("│                                                          │");
        println!("└──────────────────────────────────────────────────────────┘");
        print!("🎯 Enter role [app]: ");

        io::stdout().flush().map_err(ActrCliError::Io)?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(ActrCliError::Io)?;

        println!();

        let trimmed = input.trim().to_lowercase();
        if trimmed.is_empty() || trimmed == "app" {
            Ok(EchoRole::App)
        } else if trimmed == "service" {
            Ok(EchoRole::Service)
        } else if trimmed == "both" {
            Ok(EchoRole::Both)
        } else {
            Err(ActrCliError::InvalidProject(format!(
                "Invalid role '{trimmed}'. Use 'service', 'app' or 'both'."
            )))
        }
    }

    /// Execute initialization when role=both: generate echo-app and echo-service projects.
    async fn execute_both(
        &self,
        name: &str,
        signaling_url: &str,
        manufacturer: &str,
    ) -> Result<()> {
        let (parent_dir, _ignored_project_name) = self.resolve_project_info(name)?;

        // Determine concrete subdirectories for app and service.
        let app_dir = if parent_dir == Path::new(".") {
            PathBuf::from("echo-app")
        } else {
            parent_dir.join("echo-app")
        };
        let service_dir = if parent_dir == Path::new(".") {
            PathBuf::from("echo-service")
        } else {
            parent_dir.join("echo-service")
        };

        info!(
            "🚀 Initializing Actor-RTC echo projects: {} and {}",
            app_dir.display(),
            service_dir.display()
        );

        // Prevent overwriting existing directories.
        if app_dir.exists() || service_dir.exists() {
            return Err(ActrCliError::InvalidProject(format!(
                "Target directories '{}' or '{}' already exist. Remove them or choose a different project name.",
                app_dir.display(),
                service_dir.display()
            )));
        }

        // Check if current directory already has manifest.toml when using "."
        if parent_dir == Path::new(".") && Path::new("manifest.toml").exists() {
            return Err(ActrCliError::InvalidProject(
                "Current directory already contains an ACTR workload project (manifest.toml exists)"
                    .to_string(),
            ));
        }

        // Create parent directory if needed (for non-current-dir case).
        if parent_dir != Path::new(".") {
            std::fs::create_dir_all(&parent_dir)?;
        }

        // Normalize the signaling URL: strip trailing "/signaling/ws" (and optional "/")
        // so that each language template can append its own path suffix without duplication.
        let normalized_signaling_url = signaling_url
            .strip_suffix("/signaling/ws/")
            .or_else(|| signaling_url.strip_suffix("/signaling/ws"))
            .unwrap_or(signaling_url)
            .trim_end_matches('/')
            .to_string();

        // Build InitContext for echo-app (role=app).
        let app_context = InitContext {
            project_dir: app_dir.clone(),
            project_name: "echo-app".to_string(),
            signaling_url: normalized_signaling_url.clone(),
            template: self.template,
            is_current_dir: false,
            echo_role: Some(EchoRole::App),
            manufacturer: manufacturer.to_string(),
            is_both: true,
        };

        // Build InitContext for echo-service (role=service).
        let service_context = InitContext {
            project_dir: service_dir.clone(),
            project_name: "echo-service".to_string(),
            signaling_url: normalized_signaling_url,
            template: self.template,
            is_current_dir: false,
            echo_role: Some(EchoRole::Service),
            manufacturer: manufacturer.to_string(),
            is_both: true,
        };

        // Generate service first, then app.
        initialize::execute_initialize(self.language, &service_context).await?;
        initialize::execute_initialize(self.language, &app_context).await?;

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
                println!("│     wss://example.com/?token=${{TOKEN}}   (with auth)    │");
                println!("│                                                          │");
                println!("└──────────────────────────────────────────────────────────┘");
                print!("🎯 Enter signaling server URL [wss://actrix1.develenv.com]: ");
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
                "signaling server URL" => "wss://actrix1.develenv.com/signaling/ws",
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
}
