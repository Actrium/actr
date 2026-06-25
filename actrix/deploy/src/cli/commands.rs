//! CLI command definitions

use clap::Subcommand;
use std::path::PathBuf;

/// Available subcommands for the deployment helper
#[derive(Subcommand)]
pub enum Commands {
    /// Check system dependencies
    Deps,
    /// Install actrix from a GitHub Release tag, the latest release, or a local binary
    Install {
        /// GitHub Release tag, e.g. v0.4.3
        #[arg(long)]
        tag: Option<String>,
        /// Use the latest stable GitHub Release
        #[arg(long)]
        latest: bool,
        /// Local pre-downloaded binary file
        #[arg(long)]
        binary_path: Option<PathBuf>,
        /// SHA-256 sidecar for --binary-path (required unless --skip-verify)
        #[arg(long)]
        sha256_path: Option<PathBuf>,
        /// Version label for --binary-path / --from-local-build (e.g. v0.4.3)
        #[arg(long)]
        version: Option<String>,
        /// Skip SHA-256 verification (not safe for production)
        #[arg(long)]
        skip_verify: bool,
        /// Dev only: use the local target/release/actrix build
        #[arg(long)]
        from_local_build: bool,
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
    /// Deploy systemd service (flags optional; prompts for missing values)
    Service {
        /// Service/unit name (default: actrix)
        #[arg(long)]
        service_name: Option<String>,
        /// Installation directory
        #[arg(long)]
        install_dir: Option<PathBuf>,
        /// Configuration file path
        #[arg(long)]
        config: Option<PathBuf>,
        /// Service user
        #[arg(long)]
        user: Option<String>,
        /// Service group
        #[arg(long)]
        group: Option<String>,
        /// Overwrite an existing systemd unit (discards hardening)
        #[arg(long)]
        force_overwrite_unit: bool,
    },
    /// Uninstall the application
    Uninstall,
}
