//! Configuration file location selection page

use super::ServiceSelectionPage;
use crate::menu::framework::{
    ContentArea, Layout, LayoutComponents, Page, PageContext, PageResult, StandardLayout,
};
use anyhow::Result;
use dialoguer::{Confirm, Input, theme::ColorfulTheme};
use std::path::PathBuf;

pub struct ConfigLocationPage {
    layout: StandardLayout,
}

impl ConfigLocationPage {
    pub fn new() -> Self {
        Self {
            layout: StandardLayout,
        }
    }

    fn choose_config_location(&self) -> Result<PathBuf> {
        println!("📁 Configuration File Location");
        println!("═══════════════════════════════");
        println!();

        let default_path = PathBuf::from("/etc/actor-rtc-actrix/config.toml");
        let theme = ColorfulTheme::default();

        let config_path: String = Input::with_theme(&theme)
            .with_prompt("Configuration file path")
            .default(default_path.to_string_lossy().to_string())
            .interact_text()?;

        let path = PathBuf::from(config_path);

        // Check if file already exists
        if path.exists() {
            println!("⚠️  Configuration file already exists: {}", path.display());
            let overwrite = Confirm::with_theme(&theme)
                .with_prompt("Overwrite existing file?")
                .default(false)
                .interact()?;

            if !overwrite {
                return Err(anyhow::anyhow!("config_file_exists"));
            }
        }

        // Check if parent directory exists, create if needed
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                println!("📂 Creating configuration directory: {}", parent.display());

                // Try to create directory first without sudo
                match std::fs::create_dir_all(parent) {
                    Ok(_) => {
                        println!("✅ Directory created successfully");
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                        // Permission denied, try with sudo
                        println!("⚠️  Permission denied, creating directory with sudo...");

                        let output = std::process::Command::new("sudo")
                            .args(["mkdir", "-p", &parent.to_string_lossy()])
                            .output()
                            .map_err(|e| anyhow::anyhow!("Failed to execute sudo: {}", e))?;

                        if output.status.success() {
                            println!("✅ Directory created with sudo: {}", parent.display());
                        } else {
                            let error = String::from_utf8_lossy(&output.stderr);
                            anyhow::bail!("Failed to create directory with sudo: {}", error);
                        }
                    }
                    Err(e) => {
                        anyhow::bail!("Failed to create directory {}: {}", parent.display(), e);
                    }
                }
            }
        }

        println!("✅ Configuration file location: {}", path.display());
        println!();

        Ok(path)
    }
}

impl Page for ConfigLocationPage {
    fn title(&self) -> &str {
        "Configuration File Location"
    }

    fn render(&mut self, context: &mut PageContext) -> Result<PageResult> {
        // Build layout components
        let components = LayoutComponents::new("ActorRTC Auxiliary Services Deployment Helper")
            .with_page_title("Configuration Wizard - File Location")
            .with_operation_hint("Choose where to save the configuration file")
            .add_content(ContentArea::new().add_section(
                "Configuration File",
                vec![
                    "Choose the location for config.toml".to_string(),
                    "Default: /etc/actor-rtc-actrix/config.toml".to_string(),
                    "Directory will be created if needed".to_string(),
                ],
            ));

        // Render the layout
        self.layout.render(components);

        // Choose configuration location
        match self.choose_config_location() {
            Ok(_config_path) => {
                // Store the chosen path in context for later use
                // For now, continue to service selection
                // TODO: Pass config_path through the wizard chain
                Ok(PageResult::Navigate(Box::new(ServiceSelectionPage::new())))
            }
            Err(e) => {
                println!("❌ Error: {}", e);
                let interrupted = crate::system::press_any_key_to_with_interrupt(
                    "continue",
                    context.interrupted.clone(),
                );
                if interrupted {
                    Ok(PageResult::Stay)
                } else {
                    Ok(PageResult::Back)
                }
            }
        }
    }
}
