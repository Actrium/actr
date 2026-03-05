//! Utility functions for actr-cli

use crate::assets::FixtureAssets;
use crate::error::{ActrCliError, Result};
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;
use tokio::process::Command as TokioCommand;
use tracing::{debug, info, warn};

pub const GIT_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Execute a command and return the output
#[allow(dead_code)]
pub async fn execute_command(cmd: &str, args: &[&str], cwd: Option<&Path>) -> Result<Output> {
    debug!("Executing command: {} {}", cmd, args.join(" "));

    let mut command = TokioCommand::new(cmd);
    command.args(args);

    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }

    let output = command.output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ActrCliError::command_error(format!(
            "Command '{}' failed with exit code {:?}: {}",
            cmd,
            output.status.code(),
            stderr
        )));
    }

    Ok(output)
}

/// Execute a command and stream its output
pub async fn execute_command_streaming(cmd: &str, args: &[&str], cwd: Option<&Path>) -> Result<()> {
    info!("Running: {} {}", cmd, args.join(" "));

    let mut command = TokioCommand::new(cmd);
    command.args(args);

    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }

    let status = command.status().await?;

    if !status.success() {
        return Err(ActrCliError::command_error(format!(
            "Command '{}' failed with exit code {:?}",
            cmd,
            status.code()
        )));
    }

    Ok(())
}

/// Check if a command is available in the system PATH
pub fn command_exists(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Check if required tools are available
pub fn check_required_tools() -> Result<()> {
    let required_tools = vec![
        ("cargo", "Rust toolchain"),
        ("protoc", "Protocol Buffers compiler"),
    ];

    let mut missing_tools = Vec::new();

    for (tool, description) in required_tools {
        if !command_exists(tool) {
            missing_tools.push((tool, description));
        }
    }

    if !missing_tools.is_empty() {
        let mut error_msg = "Missing required tools:\n".to_string();
        for (tool, description) in missing_tools {
            error_msg.push_str(&format!("  - {tool} ({description})\n"));
        }
        error_msg.push_str("\nPlease install the missing tools and try again.");
        return Err(ActrCliError::command_error(error_msg));
    }

    Ok(())
}

/// Find the workspace root by looking for Cargo.toml with [workspace]
pub fn find_workspace_root() -> Result<Option<std::path::PathBuf>> {
    let mut current = std::env::current_dir()?;

    loop {
        let cargo_toml = current.join("Cargo.toml");
        if cargo_toml.exists() {
            let content = std::fs::read_to_string(&cargo_toml)?;
            if content.contains("[workspace]") {
                return Ok(Some(current));
            }
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    Ok(None)
}

/// Get the target directory for build outputs
pub fn get_target_dir(project_root: &Path) -> std::path::PathBuf {
    // Check for workspace root first
    if let Ok(Some(workspace_root)) = find_workspace_root() {
        workspace_root.join("target")
    } else {
        project_root.join("target")
    }
}

/// Convert a string to PascalCase using heck crate
pub fn to_pascal_case(input: &str) -> String {
    heck::AsPascalCase(input).to_string()
}

/// Convert a string to snake_case using heck crate
pub fn to_snake_case(input: &str) -> String {
    heck::AsSnakeCase(input).to_string()
}

/// Ensure a directory exists, creating it if necessary
#[allow(dead_code)]
pub fn ensure_dir_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        debug!("Creating directory: {}", path.display());
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Read a fixture file, falling back to embedded assets when not on disk.
pub fn read_fixture_text(fixture_path: &Path) -> Result<String> {
    if fixture_path.exists() {
        return std::fs::read_to_string(fixture_path).map_err(|error| {
            ActrCliError::Io(std::io::Error::new(
                error.kind(),
                format!(
                    "Failed to read fixture {}: {}",
                    fixture_path.display(),
                    error
                ),
            ))
        });
    }

    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let relative = fixture_path
        .strip_prefix(&fixtures_root)
        .map_err(|_| {
            ActrCliError::Io(std::io::Error::new(
                ErrorKind::NotFound,
                format!("Fixture not found: {}", fixture_path.display()),
            ))
        })?
        .to_string_lossy()
        .replace('\\', "/");

    let file = FixtureAssets::get(&relative).ok_or_else(|| {
        ActrCliError::Io(std::io::Error::new(
            ErrorKind::NotFound,
            format!("Embedded fixture not found: {}", relative),
        ))
    })?;

    let content = std::str::from_utf8(file.data.as_ref()).map_err(|error| {
        ActrCliError::Io(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("Invalid UTF-8 fixture {}: {}", relative, error),
        ))
    })?;

    Ok(content.to_string())
}

/// Fetch the latest tag from a git repository with a timeout
pub async fn fetch_latest_git_tag(url: &str, fallback: &str) -> String {
    debug!("Fetching latest tag for {}", url);

    let fetch_task = async {
        let output = TokioCommand::new("git")
            .args(["ls-remote", "--tags", "--sort=v:refname", url])
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Parse tags like "refs/tags/v0.1.10" and get the last one
                stdout
                    .lines()
                    .filter_map(|line| {
                        line.split("refs/tags/").nth(1).map(|tag| {
                            let tag = tag.trim();
                            if let Some(stripped) = tag.strip_prefix('v') {
                                stripped.to_string()
                            } else {
                                tag.to_string()
                            }
                        })
                    })
                    .rfind(|tag| !tag.contains("^{}")) // Filter out dereferenced tags
            }
            _ => None,
        }
    };

    match tokio::time::timeout(GIT_FETCH_TIMEOUT, fetch_task).await {
        Ok(Some(tag)) => {
            info!("Successfully fetched latest tag for {}: {}", url, tag);
            tag
        }
        _ => {
            warn!(
                "Failed to fetch latest tag for {} or timed out, using fallback: {}",
                url, fallback
            );
            fallback.to_string()
        }
    }
}

/// Copy a file, creating parent directories as needed
#[allow(dead_code)]
pub fn copy_file_with_dirs(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        ensure_dir_exists(parent)?;
    }
    std::fs::copy(from, to)?;
    Ok(())
}

/// Check if the current directory contains an Actr.toml file
pub fn is_actr_project() -> bool {
    Path::new("Actr.toml").exists()
}

/// Warn if not in an actr project directory
pub fn warn_if_not_actr_project() {
    if !is_actr_project() {
        warn!("Not in an Actor-RTC project directory (no Actr.toml found)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_command_exists() {
        // These commands should exist on most systems
        assert!(command_exists("ls") || command_exists("dir"));
        assert!(!command_exists("this_command_definitely_does_not_exist"));
    }

    #[test]
    fn test_ensure_dir_exists() {
        let temp_dir = TempDir::new().unwrap();
        let test_path = temp_dir.path().join("test/nested/dir");

        assert!(!test_path.exists());
        ensure_dir_exists(&test_path).unwrap();
        assert!(test_path.exists());

        // Should not fail if directory already exists
        ensure_dir_exists(&test_path).unwrap();
    }

    #[tokio::test]
    async fn test_execute_command() {
        // Test a simple command that should succeed
        let result = execute_command("echo", &["hello"], None).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("hello"));
    }
}
