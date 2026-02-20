//! CLI command definitions

use clap::Subcommand;
use std::path::PathBuf;

/// Available subcommands for the deployment helper
#[derive(Subcommand)]
pub enum Commands {
    /// Check system dependencies
    Deps,
    /// Install application files
    Install {
        /// Installation directory
        #[arg(long, default_value = "/opt/actrix")]
        install_dir: PathBuf,
        /// Binary name
        #[arg(long, default_value = "actrix")]
        binary_name: String,
        /// Skip creating symlink in /usr/local/bin
        #[arg(long)]
        no_path: bool,
    },
    /// Deploy systemd service
    Service,
    /// Uninstall the application
    Uninstall,
}
