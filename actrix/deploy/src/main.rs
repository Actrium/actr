//! Deploy helper for Actrix auxiliary services

use anyhow::Result;
use clap::Parser;

mod artifact;
mod checksum;
mod cli;
mod config;
mod release;
mod system;
mod tpl;

use artifact::Source;
use cli::{Cli, Commands};
use config::InstallConfig;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Deps) | None => system::check_dependencies(),
        Some(Commands::Install {
            tag,
            latest,
            binary_path,
            sha256_path,
            version,
            skip_verify,
            from_local_build,
            install_dir,
            no_path,
        }) => {
            let install_config = InstallConfig {
                install_dir,
                binary_name: "actrix".to_string(),
                add_to_path: !no_path,
            };
            let source = build_install_source(tag, latest, binary_path, from_local_build)?;
            // --from-local-build is dev-only: default to "local" version and
            // bypass checksum (the local build has no sidecar).
            let (version, skip_verify) = if from_local_build {
                (version.or_else(|| Some("local".to_string())), true)
            } else {
                (version, skip_verify)
            };
            system::install_from_source(&install_config, source, version, sha256_path, skip_verify)
        }
        Some(Commands::Service {
            service_name,
            install_dir,
            config,
            user,
            group,
            force_overwrite_unit,
            working_directory,
        }) => system::install_systemd_service(system::ServiceArgs {
            service_name,
            install_dir,
            config,
            user,
            group,
            force_overwrite_unit,
            working_directory,
        }),
        Some(Commands::Update {
            tag,
            latest,
            binary_path,
            sha256_path,
            version,
            skip_verify,
            install_dir,
            restart_service,
            health_url,
        }) => {
            let source = build_install_source(tag, latest, binary_path, false)?;
            system::update_service(
                install_dir,
                source,
                version,
                sha256_path,
                skip_verify,
                restart_service,
                health_url,
            )
        }
        Some(Commands::Rollback {
            to,
            install_dir,
            restart_service,
            health_url,
        }) => system::rollback_command(install_dir, to, restart_service, health_url),
        Some(Commands::Status {
            install_dir,
            service_name,
        }) => system::status_command(install_dir, service_name),
        Some(Commands::Uninstall {
            install_dir,
            service_name,
        }) => system::uninstall_application(system::UninstallArgs {
            install_dir,
            service_name,
        }),
    }
}

/// Build a binary [`Source`] from the mutually-exclusive install flags.
fn build_install_source(
    tag: Option<String>,
    latest: bool,
    binary_path: Option<std::path::PathBuf>,
    from_local_build: bool,
) -> Result<Source> {
    let mut chosen = Vec::new();
    if tag.is_some() {
        chosen.push("tag");
    }
    if latest {
        chosen.push("latest");
    }
    if binary_path.is_some() {
        chosen.push("binary-path");
    }
    if from_local_build {
        chosen.push("from-local-build");
    }
    match chosen.as_slice() {
        ["tag"] => Ok(Source::Tag(tag.unwrap())),
        ["latest"] => Ok(Source::Latest),
        ["binary-path"] => Ok(Source::BinaryPath(binary_path.unwrap())),
        ["from-local-build"] => {
            let path = system::find_local_build_binary()?;
            Ok(Source::BinaryPath(path))
        }
        _ => Err(anyhow::anyhow!(
            "specify exactly one of --tag, --latest, --binary-path, or --from-local-build"
        )),
    }
}
