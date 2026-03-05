use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{ActrCliError, Result};

const CONFIG_FILE_NAME: &str = ".protoc-plugin.toml";

#[derive(Debug, Deserialize)]
struct ProtocPluginFile {
    version: Option<u32>,
    plugins: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone)]
pub struct ProtocPluginConfig {
    path: PathBuf,
    plugins: HashMap<String, String>,
}

impl ProtocPluginConfig {
    pub fn min_version(&self, plugin: &str) -> Option<&str> {
        self.plugins.get(plugin).map(String::as_str)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn load_protoc_plugin_config(config_path: &Path) -> Result<Option<ProtocPluginConfig>> {
    let config_dir = config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let plugin_path = config_dir.join(CONFIG_FILE_NAME);
    if !plugin_path.exists() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(&plugin_path)?;
    let parsed: ProtocPluginFile = toml::from_str(&contents).map_err(|e| {
        ActrCliError::config_error(format!("Failed to parse {}: {e}", plugin_path.display()))
    })?;

    if let Some(version) = parsed.version
        && version != 1
    {
        return Err(ActrCliError::config_error(format!(
            "Unsupported .protoc-plugin.toml version {version} (expected 1)"
        )));
    }

    let plugins = parsed.plugins.unwrap_or_default();
    for (name, min_version) in &plugins {
        if min_version.trim().is_empty() {
            return Err(ActrCliError::config_error(format!(
                "Minimum version for plugin '{name}' cannot be empty"
            )));
        }
        if !is_valid_version_string(min_version) {
            return Err(ActrCliError::config_error(format!(
                "Invalid minimum version '{min_version}' for plugin '{name}'"
            )));
        }
    }

    Ok(Some(ProtocPluginConfig {
        path: plugin_path,
        plugins,
    }))
}

pub fn compare_versions(v1: &str, v2: &str) -> Ordering {
    let parse_version = |v: &str| -> Vec<u32> {
        v.split('.')
            .map(|s| s.parse::<u32>().unwrap_or(0))
            .collect()
    };

    let v1_parts = parse_version(v1);
    let v2_parts = parse_version(v2);
    let max_len = v1_parts.len().max(v2_parts.len());
    for i in 0..max_len {
        let v1_part = v1_parts.get(i).copied().unwrap_or(0);
        let v2_part = v2_parts.get(i).copied().unwrap_or(0);

        match v1_part.cmp(&v2_part) {
            Ordering::Equal => continue,
            other => return other,
        }
    }

    Ordering::Equal
}

pub fn version_is_at_least(candidate: &str, minimum: &str) -> bool {
    compare_versions(candidate, minimum) != Ordering::Less
}

fn is_valid_version_string(value: &str) -> bool {
    value.chars().all(|c| c.is_ascii_digit() || c == '.')
        && value.chars().any(|c| c.is_ascii_digit())
}
