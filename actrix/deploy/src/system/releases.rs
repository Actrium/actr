//! Versioned release switching for the `releases/<version>/actrix` +
//! `bin/actrix` symlink model.
//!
//! Owns the atomic active-symlink switch, version queries, and rollback.
//! The systemd unit (`ExecStart=.../bin/actrix`) is never touched here.

use anyhow::{Context, Result, bail};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::config::InstallConfig;

/// Atomically switch `<install-dir>/bin/actrix` to point at `target`.
///
/// Uses a temp symlink + `mv -Tf` so the active path is never missing and
/// concurrent readers see either the old or new version, never a gap.
pub fn switch_active_symlink(config: &InstallConfig, target: &Path) -> Result<()> {
    let version = version_from_release_binary(config, target)?;
    ensure_release_binary(target, &version)?;

    let link = config.binary_path();
    let tmp = config
        .bin_dir()
        .join(format!(".{}.tmp", config.binary_name));

    // Remove any stale temp link.
    let _ = Command::new("sudo")
        .args(["rm", "-f", &tmp.to_string_lossy()])
        .output();

    let out = Command::new("sudo")
        .args([
            "ln",
            "-sfn",
            &target.to_string_lossy(),
            &tmp.to_string_lossy(),
        ])
        .output()?;
    if !out.status.success() {
        bail!(
            "failed to create temp symlink: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    let out = Command::new("sudo")
        .args(["mv", "-Tf", &tmp.to_string_lossy(), &link.to_string_lossy()])
        .output()?;
    if !out.status.success() {
        bail!(
            "failed to switch active symlink: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    println!(
        "✅ Active symlink: {} -> {}",
        link.display(),
        target.display()
    );
    Ok(())
}

/// Read the current `bin/actrix` symlink target, if any.
pub fn current_target(config: &InstallConfig) -> Result<Option<PathBuf>> {
    match std::fs::read_link(config.binary_path()) {
        Ok(target) => {
            let resolved = if target.is_absolute() {
                target
            } else {
                config
                    .binary_path()
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(target)
            };
            Ok(Some(normalize_path(&resolved)))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| {
            format!(
                "failed to read active symlink {}",
                config.binary_path().display()
            )
        }),
    }
}

/// Derive the currently active version from the `bin/actrix` symlink target.
///
/// Target shape: `<install-dir>/releases/<version>/actrix` -> `<version>`.
pub fn current_version(config: &InstallConfig) -> Result<Option<String>> {
    let Some(target) = current_target(config)? else {
        return Ok(None);
    };
    if !target.exists() {
        bail!(
            "invalid active binary target {}: target does not exist",
            target.display()
        );
    }
    Ok(Some(version_from_release_binary(config, &target)?))
}

/// List installed versions (subdirectories of `releases/`), sorted.
pub fn list_versions(config: &InstallConfig) -> Result<Vec<String>> {
    let dir = config.releases_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut versions = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("failed to read releases dir {}", dir.display()))?
    {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && let Some(name) = entry.file_name().to_str()
        {
            versions.push(name.to_string());
        }
    }
    versions.sort();
    Ok(versions)
}

/// Whether a given version is installed.
pub fn has_version(config: &InstallConfig, version: &str) -> bool {
    config.release_binary_path(version).exists()
}

/// Roll the active symlink back to a previously installed version.
pub fn rollback_to(config: &InstallConfig, version: &str) -> Result<()> {
    if !has_version(config, version) {
        bail!(
            "version {version} is not installed (missing {})",
            config.release_binary_path(version).display()
        );
    }
    let target = config.release_binary_path(version);
    println!("⏪ Rolling back to {version} ...");
    switch_active_symlink(config, &target)?;
    println!("✅ Rolled back: current -> {}", target.display());
    Ok(())
}

fn version_from_release_binary(config: &InstallConfig, target: &Path) -> Result<String> {
    if target.file_name().and_then(|n| n.to_str()) != Some(config.binary_name.as_str()) {
        bail!(
            "invalid active binary target {}: expected file name '{}'",
            target.display(),
            config.binary_name
        );
    }

    let version_dir = target.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "invalid active binary target {}: missing version directory",
            target.display()
        )
    })?;
    let version = version_dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "invalid active binary target {}: version is not valid UTF-8",
                target.display()
            )
        })?;
    if !is_valid_version_label(version) {
        bail!(
            "invalid active binary target {}: version label '{version}' is not supported",
            target.display()
        );
    }

    let releases_dir = version_dir.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "invalid active binary target {}: missing releases directory",
            target.display()
        )
    })?;
    if normalize_path(releases_dir) != normalize_path(&config.releases_dir()) {
        bail!(
            "invalid active binary target {}: expected it under {}",
            target.display(),
            config.releases_dir().display()
        );
    }

    Ok(version.to_string())
}

fn ensure_release_binary(target: &Path, version: &str) -> Result<()> {
    let meta = std::fs::symlink_metadata(target).map_err(|err| {
        anyhow::anyhow!(
            "release {version} is not installed or cannot be inspected ({}): {err}",
            target.display()
        )
    })?;
    if meta.file_type().is_symlink() {
        bail!(
            "invalid release {version}: {} is a symlink; release binaries must be regular files",
            target.display()
        );
    }
    if !meta.file_type().is_file() {
        bail!(
            "invalid release {version}: {} is not a regular file",
            target.display()
        );
    }
    Ok(())
}

fn is_valid_version_label(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 128
        && version != "."
        && version != ".."
        && !version.contains("..")
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    fn cfg(dir: &Path) -> InstallConfig {
        InstallConfig {
            install_dir: dir.to_path_buf(),
            binary_name: "actrix".to_string(),
            add_to_path: false,
        }
    }

    #[test]
    fn lists_and_detects_versions() {
        let dir = std::env::temp_dir().join("actrix-deploy-releases-test");
        let _ = std::fs::remove_dir_all(&dir);
        // Create version dirs each with the actrix binary inside.
        for v in ["v0.4.3", "v0.4.4"] {
            let c = cfg(&dir);
            std::fs::create_dir_all(c.release_binary_path(v).parent().unwrap()).unwrap();
            std::fs::write(c.release_binary_path(v), b"binary").unwrap();
        }
        // A stray file should be ignored.
        std::fs::write(dir.join("releases/stray.txt"), b"x").unwrap();

        let c = cfg(&dir);
        let versions = list_versions(&c).unwrap();
        assert_eq!(versions, vec!["v0.4.3".to_string(), "v0.4.4".to_string()]);
        assert!(has_version(&c, "v0.4.3"));
        assert!(!has_version(&c, "v9.9.9"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn derives_current_version_from_valid_symlink() {
        let dir = std::env::temp_dir().join(format!(
            "actrix-deploy-current-version-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let c = cfg(&dir);
        std::fs::create_dir_all(c.release_binary_path("v1.2.3").parent().unwrap()).unwrap();
        std::fs::create_dir_all(c.bin_dir()).unwrap();
        std::fs::write(c.release_binary_path("v1.2.3"), b"binary").unwrap();
        symlink(c.release_binary_path("v1.2.3"), c.binary_path()).unwrap();

        assert_eq!(current_version(&c).unwrap(), Some("v1.2.3".to_string()));
        assert_eq!(
            current_target(&c).unwrap(),
            Some(c.release_binary_path("v1.2.3"))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_current_symlink_outside_releases() {
        let dir = std::env::temp_dir().join(format!(
            "actrix-deploy-current-version-invalid-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let c = cfg(&dir);
        std::fs::create_dir_all(c.bin_dir()).unwrap();
        let outside = dir.join("outside/actrix");
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(&outside, b"binary").unwrap();
        symlink(outside, c.binary_path()).unwrap();

        assert!(current_version(&c).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_broken_current_symlink() {
        let dir = std::env::temp_dir().join(format!(
            "actrix-deploy-current-version-broken-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let c = cfg(&dir);
        std::fs::create_dir_all(c.bin_dir()).unwrap();
        symlink(c.release_binary_path("v1.2.3"), c.binary_path()).unwrap();

        assert!(current_version(&c).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn switch_rejects_symlink_release_binary() {
        let dir = std::env::temp_dir().join(format!(
            "actrix-deploy-switch-symlink-release-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let c = cfg(&dir);
        let target = c.release_binary_path("v1.2.3");
        let outside = dir.join("outside-actrix");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::create_dir_all(c.bin_dir()).unwrap();
        std::fs::write(&outside, b"binary").unwrap();
        symlink(&outside, &target).unwrap();

        assert!(switch_active_symlink(&c, &target).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn current_version_is_none_without_active_symlink() {
        let dir = std::env::temp_dir().join(format!(
            "actrix-deploy-current-version-missing-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let c = cfg(&dir);
        assert_eq!(current_version(&c).unwrap(), None);
    }
}
