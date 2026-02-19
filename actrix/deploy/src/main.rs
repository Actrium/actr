//! Deploy helper for actor-rtc auxiliary services

use anyhow::Result;
use clap::Parser;

mod cli;
mod config;
mod system;
mod tpl;

use cli::{Cli, Commands};
use config::InstallConfig;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Deps) | None => system::check_dependencies(),
        Some(Commands::Install {
            install_dir,
            binary_name,
            no_path,
        }) => {
            let install_config = InstallConfig {
                install_dir,
                binary_name,
                add_to_path: !no_path,
            };
            system::install_application(&install_config)
        }
        Some(Commands::Service) => system::install_systemd_service(),
        Some(Commands::Uninstall) => system::uninstall_application(),
    }
}
