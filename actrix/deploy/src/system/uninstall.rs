//! Application uninstallation utilities.
//!
//! Removal is split into separately-confirmed groups so operators can remove
//! the service/binaries while preserving runtime data (db/logs/shared) and
//! configuration. Defaults preserve config and data; only the systemd unit,
//! the `releases/` binaries, and the `bin/actrix` symlink are removed by
//! default.

use anyhow::Result;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

use crate::config::InstallConfig;

const DEFAULT_SYSTEM_ACCOUNT: &str = "actrix";

/// Optional flags for `deploy uninstall`.
#[derive(Debug, Clone)]
pub struct UninstallArgs {
    pub install_dir: PathBuf,
    pub service_name: Option<String>,
}

impl Default for UninstallArgs {
    fn default() -> Self {
        Self {
            install_dir: PathBuf::from("/opt/actrix"),
            service_name: None,
        }
    }
}

/// Uninstall application with selective component removal.
pub fn uninstall_application(args: UninstallArgs) -> Result<()> {
    let service_name = args
        .service_name
        .clone()
        .unwrap_or_else(|| DEFAULT_SYSTEM_ACCOUNT.to_string());
    let service_file = format!("/etc/systemd/system/{service_name}.service");
    let config_dir = "/etc/actrix";

    let config = InstallConfig {
        install_dir: args.install_dir.clone(),
        binary_name: "actrix".to_string(),
        add_to_path: false,
    };
    let releases_dir = config.releases_dir();
    let bin_link = config.binary_path();
    let data_dirs = [config.db_dir(), config.logs_dir(), config.shared_dir()];

    println!("🔍 Checking what's installed...");
    println!("   install dir : {}", args.install_dir.display());
    println!("   service name: {service_name}");

    let mut components_found = Vec::new();
    if args.install_dir.exists() {
        components_found.push("Application files");
    }
    if std::path::Path::new(config_dir).exists() {
        components_found.push("Configuration files");
    }
    if std::path::Path::new(&service_file).exists() {
        components_found.push("Systemd service");
    }
    #[cfg(unix)]
    {
        if user_exists(DEFAULT_SYSTEM_ACCOUNT) {
            components_found.push("System user (actrix)");
        }
        if group_exists(DEFAULT_SYSTEM_ACCOUNT) {
            components_found.push("System group (actrix)");
        }
    }

    if components_found.is_empty() {
        println!("✅ No actrix components found on this system.");
        return Ok(());
    }

    println!();
    println!("Found the following components:");
    for component in &components_found {
        println!("  📦 {}", component);
    }
    println!();

    let mut removed_count = 0;

    // 1. Stop and remove systemd service
    if std::path::Path::new(&service_file).exists()
        && prompt_confirm(
            "Remove systemd service? (This will stop the service if running)",
            true,
        )?
    {
        if let Err(e) = remove_systemd_service(&service_name, &service_file) {
            println!("⚠️  Failed to remove systemd service: {}", e);
        } else {
            removed_count += 1;
        }
    }

    // 2. Remove binaries: releases/ + bin/actrix symlink (default yes)
    let has_binaries = releases_dir.exists() || bin_link.exists();
    if has_binaries
        && prompt_confirm(
            &format!(
                "Remove binaries? ({} and {})",
                releases_dir.display(),
                bin_link.display()
            ),
            true,
        )?
    {
        if let Err(e) = remove_path(&releases_dir) {
            println!("⚠️  Failed to remove {}: {}", releases_dir.display(), e);
        }
        if bin_link.exists() {
            let _ = remove_path(&bin_link);
        }
        println!("✅ Binaries removed");
        removed_count += 1;
    }

    // 3. Remove runtime data: db/ logs/ shared/ (default NO — preserve)
    let existing_data: Vec<&PathBuf> = data_dirs.iter().filter(|d| d.exists()).collect();
    if !existing_data.is_empty() {
        let names: Vec<String> = existing_data
            .iter()
            .map(|d| d.display().to_string())
            .collect();
        if prompt_confirm(
            &format!("Remove runtime data? ({})", names.join(", ")),
            false,
        )? {
            for d in &existing_data {
                if let Err(e) = remove_path(d) {
                    println!("⚠️  Failed to remove {}: {}", d.display(), e);
                }
            }
            println!("✅ Runtime data removed");
            removed_count += 1;
        } else {
            println!("ℹ️  Runtime data preserved");
        }
    }

    // 4. Remove configuration files (default NO — preserve)
    if std::path::Path::new(config_dir).exists() {
        if prompt_confirm("Remove configuration files? (/etc/actrix)", false)? {
            if let Err(e) = remove_path(&PathBuf::from(config_dir)) {
                println!("⚠️  Failed to remove configuration files: {}", e);
            } else {
                println!("✅ Configuration files removed");
                removed_count += 1;
            }
        } else {
            println!("ℹ️  Configuration files preserved");
        }
    }

    // 5. Remove system user and group
    #[cfg(unix)]
    {
        if user_exists(DEFAULT_SYSTEM_ACCOUNT)
            && prompt_confirm(
                &format!("Remove system user '{DEFAULT_SYSTEM_ACCOUNT}'?"),
                true,
            )?
        {
            if let Err(e) = remove_user(DEFAULT_SYSTEM_ACCOUNT) {
                println!("⚠️  Failed to remove user: {}", e);
            } else {
                removed_count += 1;
            }
        }
        if group_exists(DEFAULT_SYSTEM_ACCOUNT)
            && prompt_confirm(
                &format!("Remove system group '{DEFAULT_SYSTEM_ACCOUNT}'?"),
                true,
            )?
        {
            if let Err(e) = remove_group(DEFAULT_SYSTEM_ACCOUNT) {
                println!("⚠️  Failed to remove group: {}", e);
            } else {
                removed_count += 1;
            }
        }
    }

    // 6. Remove the install dir itself if now empty
    if args.install_dir.exists()
        && std::fs::read_dir(&args.install_dir)
            .map(|mut r| r.next().is_none())
            .unwrap_or(false)
    {
        let _ = remove_path(&args.install_dir);
    }

    println!();
    if removed_count > 0 {
        println!(
            "🎯 Uninstallation completed! Removed {} component(s).",
            removed_count
        );
    } else {
        println!("ℹ️  No components were removed.");
    }

    Ok(())
}

#[cfg(unix)]
fn user_exists(username: &str) -> bool {
    Command::new("id")
        .arg(username)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn group_exists(groupname: &str) -> bool {
    Command::new("getent")
        .args(["group", groupname])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn remove_systemd_service(service_name: &str, service_file: &str) -> Result<()> {
    let _ = Command::new("sudo")
        .args(["systemctl", "stop", service_name])
        .output();
    let _ = Command::new("sudo")
        .args(["systemctl", "disable", service_name])
        .output();

    let output = Command::new("sudo")
        .args(["rm", "-f", service_file])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("Failed to remove systemd service file");
    }

    let _ = Command::new("sudo")
        .args(["systemctl", "daemon-reload"])
        .output();

    println!("✅ Systemd service removed");
    Ok(())
}

fn remove_path(path: &std::path::Path) -> Result<()> {
    let output = Command::new("sudo")
        .args(["rm", "-rf", &path.to_string_lossy()])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "Failed to remove {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(unix)]
fn remove_user(username: &str) -> Result<()> {
    let output = Command::new("sudo").args(["userdel", username]).output()?;
    if output.status.success() {
        println!("✅ User '{}' removed successfully", username);
        Ok(())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to remove user '{}': {}", username, error);
    }
}

#[cfg(unix)]
fn remove_group(groupname: &str) -> Result<()> {
    let output = Command::new("sudo")
        .args(["groupdel", groupname])
        .output()?;
    if output.status.success() {
        println!("✅ Group '{}' removed successfully", groupname);
        Ok(())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        if error.contains("does not exist") {
            println!(
                "ℹ️  Group '{}' was already removed (likely when user was deleted)",
                groupname
            );
            Ok(())
        } else {
            anyhow::bail!("Failed to remove group '{}': {}", groupname, error);
        }
    }
}

fn prompt_confirm(prompt: &str, default: bool) -> Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        print!("{} [{}]: ", prompt, hint);
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let value = input.trim().to_ascii_lowercase();

        if value.is_empty() {
            return Ok(default);
        }

        match value.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("Please enter y/yes or n/no."),
        }
    }
}
