//! Process management module
//!
//! Handles PID file management and user/group switching

use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Process management utilities
pub struct ProcessManager;

impl ProcessManager {
    /// Write PID file
    pub fn write_pid_file(pid_path: Option<&str>) -> Result<Option<PathBuf>> {
        if let Some(path_str) = pid_path {
            let path = Path::new(path_str);

            // Create parent directory if it doesn't exist
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create PID file directory: {parent:?}"))?;
            }

            // Get current process ID
            let pid = std::process::id();

            // Write PID to file
            let mut file = fs::File::create(path)
                .with_context(|| format!("Failed to create PID file: {path:?}"))?;

            writeln!(file, "{pid}")
                .with_context(|| format!("Failed to write PID to file: {path:?}"))?;

            info!("PID file written: {:?} (PID: {})", path, pid);
            Ok(Some(path.to_path_buf()))
        } else {
            Ok(None)
        }
    }

    /// Remove PID file
    pub fn remove_pid_file(pid_path: Option<&PathBuf>) {
        if let Some(path) = pid_path {
            if let Err(e) = fs::remove_file(path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    warn!("Failed to remove PID file {:?}: {}", path, e);
                }
            } else {
                info!("PID file removed: {:?}", path);
            }
        }
    }

    /// Drop privileges by switching to specified user and group
    #[cfg(unix)]
    pub fn drop_privileges(user: Option<&str>, group: Option<&str>) -> Result<()> {
        #[cfg(not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "tvos",
            target_os = "watchos",
            target_os = "redox",
            target_os = "haiku"
        )))]
        use nix::unistd::setgroups;
        use nix::unistd::{Uid, setgid, setuid};

        // Get current user ID
        let current_uid = Uid::current();

        // Only root can switch users
        if !current_uid.is_root() {
            if user.is_some() || group.is_some() {
                warn!("Not running as root, cannot switch user/group");
            }
            return Ok(());
        }

        // Switch group first (while we still have privileges)
        if let Some(group_name) = group {
            info!("Switching to group: {}", group_name);

            // Look up group by name
            let group_info = nix::unistd::Group::from_name(group_name)?
                .ok_or_else(|| anyhow::anyhow!("Group '{group_name}' not found"))?;

            // Set supplementary groups to empty
            #[cfg(not(any(
                target_os = "macos",
                target_os = "ios",
                target_os = "tvos",
                target_os = "watchos",
                target_os = "redox",
                target_os = "haiku"
            )))]
            setgroups(&[]).with_context(|| "Failed to clear supplementary groups")?;

            // Set group ID
            setgid(group_info.gid)
                .with_context(|| format!("Failed to set group ID to {group_name}"))?;

            info!(
                "Successfully switched to group: {} (GID: {})",
                group_name, group_info.gid
            );
        }

        // Switch user (this must be done last as it drops privileges)
        if let Some(user_name) = user {
            info!("Switching to user: {}", user_name);

            // Look up user by name
            let user_info = nix::unistd::User::from_name(user_name)?
                .ok_or_else(|| anyhow::anyhow!("User '{user_name}' not found"))?;

            // If no group was specified, use the user's primary group
            if group.is_none() {
                #[cfg(not(any(
                    target_os = "macos",
                    target_os = "ios",
                    target_os = "tvos",
                    target_os = "watchos",
                    target_os = "redox",
                    target_os = "haiku"
                )))]
                setgroups(&[]).with_context(|| "Failed to clear supplementary groups")?;

                setgid(user_info.gid)
                    .with_context(|| format!("Failed to set primary group for user {user_name}"))?;
            }

            // Set user ID (this drops privileges)
            setuid(user_info.uid)
                .with_context(|| format!("Failed to set user ID to {user_name}"))?;

            info!(
                "Successfully switched to user: {} (UID: {})",
                user_name, user_info.uid
            );
        }

        Ok(())
    }

    /// Drop privileges on non-Unix systems (no-op)
    #[cfg(not(unix))]
    pub fn drop_privileges(_user: Option<&str>, _group: Option<&str>) -> Result<()> {
        if _user.is_some() || _group.is_some() {
            warn!("User/group switching is not supported on this platform");
        }
        Ok(())
    }
}

/// Guard to ensure PID file is removed on drop
pub struct PidFileGuard {
    path: Option<PathBuf>,
}

impl PidFileGuard {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }
}

impl Drop for PidFileGuard {
    fn drop(&mut self) {
        ProcessManager::remove_pid_file(self.path.as_ref());
    }
}
