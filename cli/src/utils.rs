//! Utility functions for actr-cli

use crate::error::{ActrCliError, Result};
use std::path::Path;
use std::process::{Command, Output};
use tokio::process::Command as TokioCommand;
use tracing::{debug, info, warn};

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

/// Ensure a directory exists, creating it if necessary
#[allow(dead_code)]
pub fn ensure_dir_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        debug!("Creating directory: {}", path.display());
        std::fs::create_dir_all(path)?;
    }
    Ok(())
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
