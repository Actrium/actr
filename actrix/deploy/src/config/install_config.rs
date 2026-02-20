//! Installation configuration for binary deployment

use std::path::PathBuf;

/// Installation configuration for binary files only
#[derive(Debug, Clone)]
pub struct InstallConfig {
    /// Installation directory (default: /opt/actrix)
    pub install_dir: PathBuf,
    /// Binary name (default: actrix)
    pub binary_name: String,
    /// Whether to create symlink in /usr/local/bin
    pub add_to_path: bool,
}

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            install_dir: PathBuf::from("/opt/actrix"),
            binary_name: "actrix".to_string(),
            add_to_path: true,
        }
    }
}

impl InstallConfig {
    /// Get the binary directory path
    pub fn bin_dir(&self) -> PathBuf {
        self.install_dir.join("bin")
    }

    /// Get the logs directory path
    pub fn logs_dir(&self) -> PathBuf {
        self.install_dir.join("logs")
    }

    /// Get the database directory path
    pub fn db_dir(&self) -> PathBuf {
        self.install_dir.join("db")
    }

    /// Get the target binary path
    pub fn binary_path(&self) -> PathBuf {
        self.bin_dir().join(&self.binary_name)
    }

    /// Get the symlink path for PATH access
    pub fn symlink_path(&self) -> PathBuf {
        PathBuf::from("/usr/local/bin").join(&self.binary_name)
    }

    /// Get all directories that need to be created for installation
    pub fn all_directories(&self) -> Vec<PathBuf> {
        vec![
            self.install_dir.clone(),
            self.bin_dir(),
            self.logs_dir(),
            self.db_dir(),
        ]
    }
}
