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
        /// WorkingDirectory for the unit (default: install-dir).
        ///
        /// Set this when the actrix config uses relative paths (certs, db,
        /// sqlite) that resolve against a directory other than the install
        /// dir, e.g. `--working-directory /opt/actr-project/actrix`. Relative
        /// runtime paths from the config are resolved against this directory
        /// and added to ReadWritePaths.
        #[arg(long)]
        working_directory: Option<PathBuf>,
    },
    /// Upgrade actrix to a new version (Release or local binary)
    Update {
        /// GitHub Release tag, e.g. v0.4.4
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
        /// Version label for --binary-path (e.g. v0.4.4)
        #[arg(long)]
        version: Option<String>,
        /// Skip SHA-256 verification (not safe for production)
        #[arg(long)]
        skip_verify: bool,
        /// Installation directory
        #[arg(long, default_value = "/opt/actrix")]
        install_dir: PathBuf,
        /// Service to restart after switching. Required so the running process
        /// and the active symlink cannot drift apart.
        #[arg(long)]
        restart_service: String,
        /// HTTP readiness endpoint to poll after restart, e.g.
        /// `http://127.0.0.1:8080/health`. Falls back to `systemctl is-active`
        /// (liveness only) when unset. Also set via `ACTRIX_HEALTH_URL`.
        #[arg(long)]
        health_url: Option<String>,
    },
    /// Roll bin/actrix back to a previously installed version
    Rollback {
        /// Version to roll back to (e.g. v0.4.3)
        #[arg(long)]
        to: String,
        /// Installation directory
        #[arg(long, default_value = "/opt/actrix")]
        install_dir: PathBuf,
        /// Service to restart after rolling back
        #[arg(long)]
        restart_service: String,
        /// HTTP readiness endpoint to poll after restart (see `update --health-url`).
        #[arg(long)]
        health_url: Option<String>,
    },
    /// Show the active version, symlink target, and installed versions
    Status {
        /// Installation directory
        #[arg(long, default_value = "/opt/actrix")]
        install_dir: PathBuf,
        /// Service to inspect for the actually running binary/version
        #[arg(long)]
        service_name: Option<String>,
    },
    /// Uninstall the application (selective; preserves data/config by default)
    Uninstall {
        /// Installation directory
        #[arg(long, default_value = "/opt/actrix")]
        install_dir: PathBuf,
        /// Service/unit name (default: actrix)
        #[arg(long)]
        service_name: Option<String>,
    },
}
