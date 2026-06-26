//! Application installation utilities

use anyhow::Result;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::dependencies::{ServiceManager, detect_service_manager};
use super::firewall::{apply_firewall, plan_firewall};
use crate::artifact::{ResolvedArtifact, Source, resolve};
use crate::config::InstallConfig;
use crate::tpl::SystemdServiceTemplate;

const DEFAULT_SERVICE_USER: &str = "actrix";
const DEFAULT_SERVICE_GROUP: &str = "actrix";

/// Install actrix from a resolved binary source.
///
/// Resolves the artifact (download + verify, or local file), writes it to
/// `releases/<version>/actrix`, atomically switches the `bin/actrix` symlink,
/// and optionally adds a PATH symlink. The systemd unit is NOT touched here.
pub fn install_from_source(
    config: &InstallConfig,
    source: Source,
    version: Option<String>,
    sha256_path: Option<PathBuf>,
    skip_verify: bool,
) -> Result<()> {
    let repo = std::env::var("ACTRIX_REPOSITORY").unwrap_or_else(|_| "Actrium/actr".to_string());
    let token = std::env::var("GITHUB_TOKEN").ok();

    let artifact = resolve(
        &source,
        version.as_deref(),
        sha256_path.as_deref(),
        skip_verify,
        &repo,
        token.as_deref(),
    )?;
    install_release(&artifact, config)
}

/// Install a resolved artifact into `releases/<version>/` and switch `bin/actrix`.
pub fn install_release(artifact: &ResolvedArtifact, config: &InstallConfig) -> Result<()> {
    validate_supported_install_dir(&config.install_dir, "installation")?;

    println!("Creating directory structure...");
    let directories = config.all_directories();
    for dir in &directories {
        create_directory_with_permissions(dir, 0o755)?;
    }
    println!("✅ Directory structure created successfully");

    // Version directory holds only the binary.
    let target = config.release_binary_path(&artifact.version);
    if let Some(parent) = target.parent() {
        create_directory_with_permissions(parent, 0o755)?;
    }

    println!("📦 Installing actrix {} ...", artifact.version);
    copy_file_with_sudo(&artifact.path, &target)?;
    set_file_permissions(&target, 0o755)?;
    println!("✅ Binary installed: {}", target.display());

    // Atomically switch the active symlink.
    super::releases::switch_active_symlink(config, &target)?;

    // Optional PATH symlink.
    if config.add_to_path {
        add_to_path(&config.binary_path(), &config.symlink_path())?;
    }

    // Clean up the private download directory (binary + sidecar).
    if artifact.is_temp {
        if let Some(dir) = &artifact.temp_dir {
            let _ = std::fs::remove_dir_all(dir);
        } else {
            let _ = std::fs::remove_file(&artifact.path);
        }
    }

    println!();
    println!("📁 Installation directories:");
    for dir in &directories {
        println!("  - {}", dir.display());
    }
    println!();
    println!(
        "✅ Installation completed: {} -> {}",
        config.binary_path().display(),
        target.display()
    );

    Ok(())
}

/// Upgrade actrix to a new version, optionally restarting a service.
///
/// Resolves + verifies the artifact, installs it to `releases/<version>/`,
/// switches `bin/actrix`, and — if `restart_service` is given — restarts that
/// service and waits for it to become active. On restart failure the active
/// symlink is rolled back to the previous version and the old service
/// restarted. The systemd unit is never modified.
pub fn update_service(
    install_dir: PathBuf,
    source: Source,
    version: Option<String>,
    sha256_path: Option<PathBuf>,
    skip_verify: bool,
    restart_service: Option<String>,
    health_url: Option<String>,
) -> Result<()> {
    let config = InstallConfig {
        install_dir: install_dir.clone(),
        binary_name: "actrix".to_string(),
        add_to_path: false,
    };
    validate_supported_install_dir(&config.install_dir, "update")?;
    let svc = restart_service.as_deref();
    if let Some(name) = svc {
        validate_service_name(name)?;
    }

    let repo = std::env::var("ACTRIX_REPOSITORY").unwrap_or_else(|_| "Actrium/actr".to_string());
    let token = std::env::var("GITHUB_TOKEN").ok();
    let prev = super::releases::current_version(&config)?;

    let artifact = resolve(
        &source,
        version.as_deref(),
        sha256_path.as_deref(),
        skip_verify,
        &repo,
        token.as_deref(),
    )?;
    install_release(&artifact, &config)?;

    if let Some(svc) = &restart_service {
        let wait = super::service::health_wait_seconds();
        let health = resolve_health_url(health_url.as_deref());
        if let Err(err) = super::service::restart(svc)
            .and_then(|_| super::service::wait_ready(svc, wait, health.as_deref()))
        {
            println!("❌ Service did not come up: {err}");
            match &prev {
                Some(prev_ver) => {
                    // rollback_to() logs the rollback step itself.
                    let _ = super::releases::rollback_to(&config, prev_ver);
                    let _ = super::service::restart(svc);
                    anyhow::bail!(
                        "update to {} failed: service '{svc}' did not become active; rolled back to {prev_ver}",
                        artifact.version
                    );
                }
                None => {
                    anyhow::bail!(
                        "update to {} failed: service '{svc}' did not become active and no previous version exists to roll back to",
                        artifact.version
                    );
                }
            }
        } else {
            println!("✅ Service '{svc}' active on version {}", artifact.version);
        }
    } else {
        println!("ℹ️  No --restart-service given; symlink switched, service not restarted.");
    }

    Ok(())
}

/// Roll the active symlink back to a previously installed version.
pub fn rollback_command(
    install_dir: PathBuf,
    to_version: String,
    restart_service: Option<String>,
    health_url: Option<String>,
) -> Result<()> {
    let config = InstallConfig {
        install_dir,
        binary_name: "actrix".to_string(),
        add_to_path: false,
    };
    validate_supported_install_dir(&config.install_dir, "rollback")?;
    if let Some(name) = restart_service.as_deref() {
        validate_service_name(name)?;
    }
    super::releases::rollback_to(&config, &to_version)?;
    if let Some(svc) = &restart_service {
        super::service::restart(svc)?;
        let wait = super::service::health_wait_seconds();
        let health = resolve_health_url(health_url.as_deref());
        super::service::wait_ready(svc, wait, health.as_deref())?;
        println!("✅ Service '{svc}' active on version {to_version}");
    }
    Ok(())
}

/// Resolve a readiness-probe URL from an explicit arg or `ACTRIX_HEALTH_URL`.
///
/// When set, `update`/`rollback` poll this HTTP endpoint (e.g.
/// `http://127.0.0.1:8080/health`) instead of relying on `systemctl is-active`
/// alone, so a process that is alive but not actually serving traffic is still
/// treated as not-yet-ready and triggers rollback.
fn resolve_health_url(explicit: Option<&str>) -> Option<String> {
    explicit
        .map(|s| s.to_string())
        .or_else(|| std::env::var("ACTRIX_HEALTH_URL").ok())
        .filter(|s| !s.trim().is_empty())
}

/// Validate a systemd unit/service name.
///
/// Rejects path separators, whitespace, and newlines so a crafted
/// `--service-name` (e.g. `../foo`) cannot escape the unit directory or inject
/// directives when interpolated into a path.
pub(crate) fn validate_service_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("service name must not be empty");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        anyhow::bail!(
            "invalid service name '{name}': only letters, digits, '.', '_', and '-' are allowed"
        );
    }
    Ok(())
}

/// Print the active version, symlink target, and installed versions.
pub fn status_command(install_dir: PathBuf) -> Result<()> {
    let config = InstallConfig {
        install_dir,
        binary_name: "actrix".to_string(),
        add_to_path: false,
    };
    println!("Install dir:    {}", config.install_dir.display());
    match super::releases::current_version(&config)? {
        Some(v) => println!("Current version: {v}"),
        None => println!("Current version: (none — no active symlink)"),
    }
    match super::releases::current_target(&config)? {
        Some(t) => println!("{} -> {}", config.binary_path().display(), t.display()),
        None => println!("{} -> (missing)", config.binary_path().display()),
    }
    let versions = super::releases::list_versions(&config)?;
    if versions.is_empty() {
        println!("Installed versions: (none)");
    } else {
        println!("Installed versions: {}", versions.join(", "));
    }
    Ok(())
}

/// Locate the local `target/release/actrix` build (dev `--from-local-build` only).
pub fn find_local_build_binary() -> Result<PathBuf> {
    find_actrix_binary()
}

/// Deploy application as systemd service
/// Optional flags for `deploy service` (fully non-interactive when all set).
#[derive(Debug, Default, Clone)]
pub struct ServiceArgs {
    pub service_name: Option<String>,
    pub install_dir: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub user: Option<String>,
    pub group: Option<String>,
    pub force_overwrite_unit: bool,
    pub working_directory: Option<PathBuf>,
}

/// Deploy application as systemd service.
///
/// Accepts optional non-interactive flags; any missing value is prompted for.
/// The service/unit name is decoupled from the binary name (always `actrix`)
/// so multiple instances can run under distinct unit names on one host.
pub fn install_systemd_service(args: ServiceArgs) -> Result<()> {
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
    let config_path = configure_config_path(args.config)?;

    // Configure systemd service
    let install_config = configure_service_settings(args.install_dir)?;
    let service_name = configure_service_name(args.service_name)?;
    let (service_user, service_group) = configure_service_user(args.user, args.group)?;

    // WorkingDirectory defaults to the install dir; override with --working-directory
    // when the actrix config resolves relative paths (certs/db/sqlite) elsewhere.
    let working_directory = args
        .working_directory
        .unwrap_or_else(|| install_config.install_dir.clone());

    // Verify critical files exist before creating service
    verify_deployment_files(&install_config, &config_path)?;

    // Generate firewall changes and let user choose apply/skip
    configure_firewall_step(&config_path)?;

    // Create systemd service
    let service_template =
        SystemdServiceTemplate::new(install_config, config_path, service_name, working_directory);
    service_template.generate_service_file(
        &service_user,
        &service_group,
        args.force_overwrite_unit,
    )?;

    Ok(())
}

/// Resolve the service/unit name from a flag or prompt.
fn configure_service_name(opt: Option<String>) -> Result<String> {
    let name = match opt {
        Some(name) => name,
        None => prompt_text("Service name", "actrix")?,
    };
    validate_service_name(&name)?;
    Ok(name)
}

/// Configure configuration file path (flag or prompt).
fn configure_config_path(opt: Option<PathBuf>) -> Result<PathBuf> {
    println!("📁 Configuration File Path");
    println!("══════════════════════════");
    println!("Specify the configuration file path for the service:");
    println!();

    let path = match opt {
        Some(p) => p,
        None => PathBuf::from(prompt_text(
            "Configuration file path",
            "/etc/actrix/config.toml",
        )?),
    };

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

/// Configure systemd service settings (flag or prompt for install dir).
fn configure_service_settings(install_dir_opt: Option<PathBuf>) -> Result<InstallConfig> {
    println!("📋 Systemd Service Configuration");
    println!("═══════════════════════════════");
    println!("Configure service settings (press Enter for defaults):");
    println!();

    let default_config = InstallConfig::default();

    // Installation directory
    let install_dir = match install_dir_opt {
        Some(d) => d,
        None => PathBuf::from(prompt_text(
            "Installation directory",
            &default_config.install_dir.to_string_lossy(),
        )?),
    };
    validate_supported_install_dir(&install_dir, "service deployment")?;

    println!();

    // Binary name is always "actrix" (the actual binary in bin/); the unit name
    // is configured separately via --service-name.
    Ok(InstallConfig {
        install_dir,
        binary_name: default_config.binary_name,
        add_to_path: false, // Not relevant for systemd service
    })
}

/// Configure systemd service user and group (flags or prompt).
fn configure_service_user(
    user_opt: Option<String>,
    group_opt: Option<String>,
) -> Result<(String, String)> {
    println!("👤 Service User Configuration");
    println!("════════════════════════════");
    println!();

    // Service user
    let service_user = match user_opt {
        Some(u) => u,
        None => prompt_text("Service user", DEFAULT_SERVICE_USER)?,
    };

    // Service group
    let service_group = match group_opt {
        Some(g) => g,
        None => prompt_text("Service group", DEFAULT_SERVICE_GROUP)?,
    };

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
            create_system_group(&service_group, &service_user)?;
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
            "/opt/actrix",
            "--no-create-home",
            "--shell",
            "/usr/sbin/nologin",
            "--comment",
            "actrix service user",
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
fn create_system_group(groupname: &str, username: &str) -> Result<()> {
    println!("👥 Creating system group: {}", groupname);

    let output = Command::new("sudo")
        .args(["groupadd", "--system", groupname])
        .output()?;

    if output.status.success() {
        println!("✅ System group '{}' created", groupname);

        // Add user to group if both exist
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

fn configure_firewall_step(config_path: &Path) -> Result<()> {
    println!("🔥 Firewall Configuration");
    println!("════════════════════════");

    let preview = match plan_firewall(config_path) {
        Ok(Some(preview)) => preview,
        Ok(None) => {
            println!(
                "ℹ️  No external listener ports detected from config; skipping firewall step."
            );
            println!();
            return Ok(());
        }
        Err(error) => {
            println!(
                "⚠️  Failed to build firewall plan from config (skipping firewall step): {}",
                error
            );
            println!();
            return Ok(());
        }
    };

    println!(
        "Detected firewall manager: {}{}",
        preview.manager_name,
        if preview.manager_active {
            ""
        } else {
            " (inactive/not-running)"
        }
    );
    println!("Planned inbound rules:");
    for rule in &preview.rules {
        println!("  • {}", rule);
    }
    println!();

    if !preview.commands.is_empty() {
        println!("Generated commands:");
        for cmd in &preview.commands {
            println!("  {}", cmd);
        }
        println!();
    }

    if !preview.supported {
        println!("⚠️  No supported firewall manager found (supported: ufw, firewalld).");
        println!("ℹ️  Continue service deployment without auto-applying firewall rules.");
        println!();
        return Ok(());
    }

    let apply_now = prompt_confirm("Apply generated firewall configuration now?", false)?;
    if apply_now {
        apply_firewall(config_path)?;
        println!("✅ Firewall rules applied");
    } else {
        println!("⏭️  Firewall configuration skipped by user choice");
    }
    println!();

    Ok(())
}

pub(crate) fn validate_supported_install_dir(install_dir: &Path, operation: &str) -> Result<()> {
    let normalized = normalize_install_dir(install_dir)?;

    // Reject the filesystem root and any path still containing `..` after
    // normalization. Without this, `--install-dir /opt/actrix/../..` (which
    // resolves to `/`) would let a later `rm -rf` walk out of the intended
    // tree — particularly dangerous in `uninstall`.
    if normalized == Path::new("/") {
        anyhow::bail!(
            "Unsupported installation directory for {}: '/'. \
             Refusing to operate on the filesystem root.",
            operation
        );
    }
    if normalized.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        anyhow::bail!(
            "Unsupported installation directory for {}: '{}'. \
             Paths containing '..' are rejected; use a literal path such as '/opt/actrix'.",
            operation,
            normalized.display()
        );
    }

    if normalized.starts_with(Path::new("/home")) {
        anyhow::bail!(
            "Unsupported installation directory for {}: '{}'. \
             Paths under '/home' are blocked for service hardening consistency. \
             Use a root-owned path such as '/opt/actrix'.",
            operation,
            normalized.display()
        );
    }

    if normalized.starts_with(Path::new("/tmp")) {
        anyhow::bail!(
            "Unsupported installation directory for {}: '{}'. \
             Paths under '/tmp' are blocked for service hardening consistency. \
             Use a persistent path such as '/opt/actrix'.",
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
    // Copy to a temp file beside the destination, then atomically rename it
    // into place. A plain `cp` over a currently-running executable fails with
    // ETXTBSY ("Text file busy"); `mv` (rename) over a running binary is
    // allowed because the running process retains the old inode. This also
    // makes the replacement atomic for concurrent readers.
    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    let file_name = dst.file_name().and_then(|n| n.to_str()).unwrap_or("binary");
    let tmp = parent.join(format!(".{file_name}.deploy-tmp-{}", std::process::id()));

    let cp = Command::new("sudo")
        .args(["cp", &src.to_string_lossy(), &tmp.to_string_lossy()])
        .output()?;
    if !cp.status.success() {
        let error = String::from_utf8_lossy(&cp.stderr);
        let _ = Command::new("sudo")
            .args(["rm", "-f", &tmp.to_string_lossy()])
            .output();
        anyhow::bail!(
            "Failed to copy file from {} to {}: {}",
            src.display(),
            dst.display(),
            error
        );
    }

    let mv = Command::new("sudo")
        .args(["mv", "-f", &tmp.to_string_lossy(), &dst.to_string_lossy()])
        .output()?;
    if !mv.status.success() {
        let error = String::from_utf8_lossy(&mv.stderr);
        let _ = Command::new("sudo")
            .args(["rm", "-f", &tmp.to_string_lossy()])
            .output();
        anyhow::bail!(
            "Failed to move file into place {} -> {}: {}",
            tmp.display(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_name_rejects_traversal_and_whitespace() {
        assert!(validate_service_name("actrix").is_ok());
        assert!(validate_service_name("actrix-2.f").is_ok());
        assert!(validate_service_name("../x").is_err());
        assert!(validate_service_name("a b").is_err());
        assert!(validate_service_name("a\nb").is_err());
        assert!(validate_service_name("").is_err());
        assert!(validate_service_name("a/b").is_err());
    }

    #[test]
    fn install_dir_rejects_root_and_parent_refs() {
        assert!(validate_supported_install_dir(Path::new("/opt/actrix"), "test").is_ok());
        assert!(validate_supported_install_dir(Path::new("/"), "test").is_err());
        assert!(validate_supported_install_dir(Path::new("/opt/actrix/../.."), "test").is_err());
        assert!(validate_supported_install_dir(Path::new("/home/x/actrix"), "test").is_err());
        assert!(validate_supported_install_dir(Path::new("/tmp/actrix"), "test").is_err());
    }
}
