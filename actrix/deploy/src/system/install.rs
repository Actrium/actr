//! Application installation utilities

use anyhow::Result;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::InstallConfig;
use crate::tpl::SystemdServiceTemplate;
use super::dependencies::{detect_service_manager, ServiceManager};

/// Install application files to system directories
pub fn install_application(config: &InstallConfig) -> Result<()> {
    validate_supported_install_dir(&config.install_dir, "installation")?;

    println!("Creating directory structure...");

    // Create installation directories
    let directories = config.all_directories();
    for dir in &directories {
        create_directory_with_permissions(dir, 0o755)?;
    }

    println!("✅ Directory structure created successfully");

    // Find and copy actrix binary
    println!("Locating actrix binary...");
    let source_binary = find_actrix_binary()?;
    println!("📦 Found source binary: {}", source_binary.display());

    let target_binary = config.binary_path();
    copy_file_with_sudo(&source_binary, &target_binary)?;

    // Make binary executable
    set_file_permissions(&target_binary, 0o755)?;
    println!("✅ Actrix binary installed to {}", target_binary.display());

    // Add symlink to PATH if requested
    if config.add_to_path {
        add_to_path(&target_binary, &config.symlink_path())?;
    }

    println!();
    println!("📁 Installation directories:");
    for dir in &directories {
        println!("  - {}", dir.display());
    }
    println!();
    println!("✅ Application installation completed");

    Ok(())
}

/// Deploy application as systemd service
pub fn install_systemd_service() -> Result<()> {
    match detect_service_manager() {
        ServiceManager::Systemd => {
            println!("✅ Service manager detected: systemd");
        }
        manager => {
            anyhow::bail!(
                "Unsupported service manager environment: {}. \
                 `deploy service` currently supports only systemd. \
                 Use manual process management on this host.",
                manager.as_str()
            );
        }
    }

    println!();

    // Configure configuration file path
    let config_path = configure_config_path()?;

    // Configure systemd service
    let install_config = configure_service_settings()?;
    let (service_user, service_group) = configure_service_user()?;

    // Verify critical files exist before creating service
    verify_deployment_files(&install_config, &config_path)?;

    // Create systemd service
    let service_template = SystemdServiceTemplate::new(install_config, config_path);
    service_template.generate_service_file(&service_user, &service_group)?;

    Ok(())
}

/// Configure configuration file path
fn configure_config_path() -> Result<PathBuf> {
    println!("📁 Configuration File Path");
    println!("══════════════════════════");
    println!("Specify the configuration file path for the service:");
    println!();

    let config_path = prompt_text(
        "Configuration file path",
        "/etc/actor-rtc-actrix/config.toml",
    )?;

    let path = PathBuf::from(config_path);

    // Check if file exists
    if !path.exists() {
        println!("⚠️  Configuration file does not exist: {}", path.display());
        let confirm = prompt_confirm(
            "Continue with deployment? (you'll need to create the config later)",
            true,
        )?;

        if !confirm {
            anyhow::bail!("Deployment cancelled - configuration file required");
        }
    } else {
        println!("✅ Configuration file found: {}", path.display());
    }

    println!();
    Ok(path)
}

/// Configure systemd service settings
fn configure_service_settings() -> Result<InstallConfig> {
    println!("📋 Systemd Service Configuration");
    println!("═══════════════════════════════");
    println!("Configure service settings (press Enter for defaults):");
    println!();

    let default_config = InstallConfig::default();

    // Installation directory
    let install_dir = prompt_text(
        "Installation directory",
        &default_config.install_dir.to_string_lossy(),
    )?;
    let install_dir = PathBuf::from(install_dir);
    validate_supported_install_dir(&install_dir, "service deployment")?;

    // Binary name
    let binary_name = prompt_text("Service/binary name", &default_config.binary_name)?;

    println!();

    Ok(InstallConfig {
        install_dir,
        binary_name,
        add_to_path: false, // Not relevant for systemd service
    })
}

/// Configure systemd service user and group
fn configure_service_user() -> Result<(String, String)> {
    println!("👤 Service User Configuration");
    println!("════════════════════════════");
    println!();

    // Service user
    let service_user = prompt_text("Service user", "actor-rtc")?;

    // Service group
    let service_group = prompt_text("Service group", "actor-rtc")?;

    // Check if user exists
    if !user_exists(&service_user) {
        println!("⚠️  User '{}' does not exist", service_user);
        let create_user = prompt_confirm("Create system user?", true)?;

        if create_user {
            create_system_user(&service_user)?;
        } else {
            println!("❌ Service deployment requires the user to exist");
            anyhow::bail!(
                "User '{}' does not exist and creation was declined",
                service_user
            );
        }
    }

    // Check if group exists
    if !group_exists(&service_group) {
        println!("⚠️  Group '{}' does not exist", service_group);
        let create_group = prompt_confirm("Create system group?", true)?;

        if create_group {
            create_system_group(&service_group)?;
        } else {
            println!("❌ Service deployment requires the group to exist");
            anyhow::bail!(
                "Group '{}' does not exist and creation was declined",
                service_group
            );
        }
    }

    println!();
    Ok((service_user, service_group))
}

/// Check if a user exists
fn user_exists(username: &str) -> bool {
    Command::new("id")
        .arg(username)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Check if a group exists  
fn group_exists(groupname: &str) -> bool {
    Command::new("getent")
        .args(["group", groupname])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Create a system user
fn create_system_user(username: &str) -> Result<()> {
    println!("👤 Creating system user: {}", username);

    let output = Command::new("sudo")
        .args([
            "useradd",
            "--system",
            "--home-dir",
            "/opt/actor-rtc-actrix",
            "--no-create-home",
            "--shell",
            "/usr/sbin/nologin",
            "--comment",
            "actor-rtc actrix service user",
            username,
        ])
        .output()?;

    if output.status.success() {
        println!("✅ System user '{}' created", username);
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to create user '{}': {}", username, error);
    }

    Ok(())
}

/// Create a system group
fn create_system_group(groupname: &str) -> Result<()> {
    println!("👥 Creating system group: {}", groupname);

    let output = Command::new("sudo")
        .args(["groupadd", "--system", groupname])
        .output()?;

    if output.status.success() {
        println!("✅ System group '{}' created", groupname);

        // Add user to group if both exist
        // Note: This assumes the user was just created above
        let username = "actor-rtc"; // Default user name
        if user_exists(username) {
            let output = Command::new("sudo")
                .args(["usermod", "-a", "-G", groupname, username])
                .output()?;

            if output.status.success() {
                println!("✅ User '{}' added to group '{}'", username, groupname);
            }
        }
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to create group '{}': {}", groupname, error);
    }

    Ok(())
}

/// Find the actrix binary in the project structure
fn find_actrix_binary() -> Result<PathBuf> {
    let search_bases = [
        std::env::current_exe()?.parent().unwrap().to_path_buf(),
        std::env::current_dir()?,
    ];

    let target_paths = [
        "target/release/actrix",
        "crates/actrixd/target/release/actrix",
    ];
    let max_steps_up = 4; // Maximum directory levels to go up

    for base_dir in &search_bases {
        for target_path in &target_paths {
            for steps_up in 0..max_steps_up {
                let mut search_dir = base_dir.clone();

                // Go up the directory tree
                for _ in 0..steps_up {
                    if let Some(parent) = search_dir.parent() {
                        search_dir = parent.to_path_buf();
                    } else {
                        break; // Can't go up anymore
                    }
                }

                let candidate = search_dir.join(target_path);
                if candidate.exists() && candidate.is_file() {
                    return Ok(candidate.canonicalize()?);
                }
            }
        }
    }

    anyhow::bail!(
        "Could not find actrix binary. Please ensure it's built with:\n  \
         cargo build --release --bin actrix"
    )
}

fn add_to_path(binary_path: &Path, symlink_path: &Path) -> Result<()> {
    // Remove existing symlink if it exists
    let _ = Command::new("sudo")
        .args(["rm", "-f", &symlink_path.to_string_lossy()])
        .output();

    // Create new symlink
    let output = Command::new("sudo")
        .args([
            "ln",
            "-s",
            &binary_path.to_string_lossy(),
            &symlink_path.to_string_lossy(),
        ])
        .output()?;

    if output.status.success() {
        println!(
            "✅ Created symlink: {} -> {}",
            symlink_path.display(),
            binary_path.display()
        );
        println!(
            "   The '{}' command is now available in your PATH",
            symlink_path.file_name().unwrap().to_string_lossy()
        );
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        println!("⚠️  Warning: Failed to create symlink to PATH: {}", error);
        println!(
            "   You can manually add {} to your PATH",
            binary_path.display()
        );
    }

    Ok(())
}

/// Verify that critical files exist before deploying service
fn verify_deployment_files(install_config: &InstallConfig, config_path: &Path) -> Result<()> {
    println!("🔍 Verifying deployment files...");
    println!();
    validate_supported_install_dir(&install_config.install_dir, "service deployment")?;

    let mut missing_files = Vec::new();

    // Check binary file
    let binary_path = install_config.binary_path();
    if binary_path.exists() {
        println!("✅ Binary file found: {}", binary_path.display());
    } else {
        println!("❌ Binary file missing: {}", binary_path.display());
        missing_files.push(format!("Binary: {}", binary_path.display()));
    }

    // Check configuration file
    if config_path.exists() {
        println!("✅ Configuration file found: {}", config_path.display());
    } else {
        println!("❌ Configuration file missing: {}", config_path.display());
        missing_files.push(format!("Config: {}", config_path.display()));
    }

    // Check if install directory exists
    let install_dir = &install_config.install_dir;
    if install_dir.exists() {
        println!("✅ Installation directory found: {}", install_dir.display());
    } else {
        println!(
            "❌ Installation directory missing: {}",
            install_dir.display()
        );
        missing_files.push(format!("Install dir: {}", install_dir.display()));
    }

    if !missing_files.is_empty() {
        println!();
        println!("⚠️  Missing critical files for service deployment:");
        for file in &missing_files {
            println!("   • {}", file);
        }
        println!();
        println!("Suggestions:");

        if !binary_path.exists() {
            println!("   • Run application installation first to copy the binary");
            println!("   • Or build the project: cargo build --release --bin actrix");
        }

        if !config_path.exists() {
            println!("   • Create the configuration file manually (default path shown above)");
        }

        if !install_dir.exists() {
            println!("   • Run application installation first to create directories");
        }

        anyhow::bail!("Cannot deploy service - critical files missing");
    }

    println!("✅ All critical files verified");
    println!();
    Ok(())
}

fn validate_supported_install_dir(install_dir: &Path, operation: &str) -> Result<()> {
    let normalized = normalize_install_dir(install_dir)?;

    if normalized.starts_with(Path::new("/home")) {
        anyhow::bail!(
            "Unsupported installation directory for {}: '{}'. \
             Paths under '/home' are blocked for service hardening consistency. \
             Use a root-owned path such as '/opt/actor-rtc-actrix'.",
            operation,
            normalized.display()
        );
    }

    if normalized.starts_with(Path::new("/tmp")) {
        anyhow::bail!(
            "Unsupported installation directory for {}: '{}'. \
             Paths under '/tmp' are blocked for service hardening consistency. \
             Use a persistent path such as '/opt/actor-rtc-actrix'.",
            operation,
            normalized.display()
        );
    }

    Ok(())
}

fn normalize_install_dir(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn create_directory_with_permissions(path: &Path, mode: u32) -> Result<()> {
    if path.exists() {
        return Ok(());
    }

    if std::fs::create_dir_all(path).is_err() {
        let output = Command::new("sudo")
            .args(["mkdir", "-p", &path.to_string_lossy()])
            .output()?;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to create directory {}: {}", path.display(), error);
        }
    }

    set_file_permissions(path, mode)?;
    Ok(())
}

fn copy_file_with_sudo(src: &Path, dst: &Path) -> Result<()> {
    let output = Command::new("sudo")
        .args(["cp", &src.to_string_lossy(), &dst.to_string_lossy()])
        .output()?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to copy file from {} to {}: {}",
            src.display(),
            dst.display(),
            error
        );
    }

    Ok(())
}

fn set_file_permissions(path: &Path, mode: u32) -> Result<()> {
    let mode_str = format!("{:o}", mode);
    let output = Command::new("sudo")
        .args(["chmod", &mode_str, &path.to_string_lossy()])
        .output()?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to set permissions on {}: {}", path.display(), error);
    }

    Ok(())
}

fn prompt_text(prompt: &str, default: &str) -> Result<String> {
    print!("{} [{}]: ", prompt, default);
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let value = input.trim();
    if value.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(value.to_string())
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
