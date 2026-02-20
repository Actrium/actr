//! Application uninstallation utilities

use anyhow::Result;
use std::io::{self, Write};
use std::process::Command;

const DEFAULT_SYSTEM_ACCOUNT: &str = "actrix";

/// Uninstall application with selective component removal
pub fn uninstall_application() -> Result<()> {
    println!("🔍 Checking what's installed...");

    // Check what's currently installed
    let install_dir = "/opt/actrix";
    let config_dir = "/etc/actrix";
    let service_file = "/etc/systemd/system/actrix.service";

    let mut components_found = Vec::new();

    if std::path::Path::new(install_dir).exists() {
        components_found.push("Application files");
    }

    if std::path::Path::new(config_dir).exists() {
        components_found.push("Configuration files");
    }

    if std::path::Path::new(service_file).exists() {
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

    // Selective removal
    let mut removed_count = 0;

    // 1. Stop and remove systemd service
    if std::path::Path::new(service_file).exists()
        && prompt_confirm(
            "Remove systemd service? (This will stop the service if running)",
            true,
        )?
    {
        if let Err(e) = remove_systemd_service() {
            println!("⚠️  Failed to remove systemd service: {}", e);
        } else {
            removed_count += 1;
        }
    }

    // 2. Remove application files
    if std::path::Path::new(install_dir).exists()
        && prompt_confirm("Remove application files? (/opt/actrix)", true)?
    {
        if let Err(e) = remove_directory(install_dir) {
            println!("⚠️  Failed to remove application files: {}", e);
        } else {
            println!("✅ Application files removed");
            removed_count += 1;
        }
    }

    // 3. Remove configuration files (optional)
    if std::path::Path::new(config_dir).exists() {
        if prompt_confirm("Remove configuration files? (/etc/actrix)", false)? {
            if let Err(e) = remove_directory(config_dir) {
                println!("⚠️  Failed to remove configuration files: {}", e);
            } else {
                println!("✅ Configuration files removed");
                removed_count += 1;
            }
        } else {
            println!("ℹ️  Configuration files preserved");
        }
    }

    // 4. Remove system user and group
    #[cfg(unix)]
    {
        if user_exists(DEFAULT_SYSTEM_ACCOUNT) {
            if prompt_confirm(
                &format!("Remove system user '{DEFAULT_SYSTEM_ACCOUNT}'?"),
                true,
            )? {
                if let Err(e) = remove_user(DEFAULT_SYSTEM_ACCOUNT) {
                    println!("⚠️  Failed to remove user: {}", e);
                } else {
                    removed_count += 1;
                }
            } else {
                println!("ℹ️  System user preserved");
            }
        }

        if group_exists(DEFAULT_SYSTEM_ACCOUNT) {
            if prompt_confirm(
                &format!("Remove system group '{DEFAULT_SYSTEM_ACCOUNT}'?"),
                true,
            )? {
                if let Err(e) = remove_group(DEFAULT_SYSTEM_ACCOUNT) {
                    println!("⚠️  Failed to remove group: {}", e);
                } else {
                    removed_count += 1;
                }
            } else {
                println!("ℹ️  System group preserved");
            }
        }
    }

    // Summary
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

fn remove_systemd_service() -> Result<()> {
    let service_name = "actrix";
    let service_file = "/etc/systemd/system/actrix.service";

    // Stop the service if running
    let _ = Command::new("sudo")
        .args(["systemctl", "stop", service_name])
        .output();

    // Disable the service
    let _ = Command::new("sudo")
        .args(["systemctl", "disable", service_name])
        .output();

    // Remove service file
    let output = Command::new("sudo")
        .args(["rm", "-f", service_file])
        .output()?;

    if !output.status.success() {
        anyhow::bail!("Failed to remove systemd service file");
    }

    // Reload systemd
    let _ = Command::new("sudo")
        .args(["systemctl", "daemon-reload"])
        .output();

    println!("✅ Systemd service removed");
    Ok(())
}

fn remove_directory(path: &str) -> Result<()> {
    let output = Command::new("sudo").args(["rm", "-rf", path]).output()?;

    if !output.status.success() {
        anyhow::bail!("Failed to remove directory: {}", path);
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

        // If group doesn't exist (maybe removed when user was deleted), treat as success
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
