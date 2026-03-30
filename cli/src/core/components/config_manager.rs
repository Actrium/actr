use actr_config::Config;
use actr_protocol::ActrTypeExt;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::fs;
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

use crate::core::{ConfigBackup, ConfigManager, ConfigValidation, DependencySpec};
use actr_config::ConfigParser;

pub struct TomlConfigManager {
    config_path: PathBuf,
    project_root: PathBuf,
}

impl TomlConfigManager {
    pub fn new<P: Into<PathBuf>>(config_path: P) -> Self {
        let config_path = config_path.into();
        let project_root = resolve_project_root(&config_path);
        Self {
            config_path,
            project_root,
        }
    }

    async fn read_config_string(&self, path: &Path) -> Result<String> {
        fs::read_to_string(path)
            .await
            .with_context(|| format!("Failed to read config file: {}", path.display()))
    }

    async fn write_config_string(&self, path: &Path, contents: &str) -> Result<()> {
        fs::write(path, contents)
            .await
            .with_context(|| format!("Failed to write config file: {}", path.display()))
    }

    fn build_backup_path(&self) -> Result<PathBuf> {
        let file_name = self
            .config_path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Config path is missing file name"))?
            .to_string_lossy();
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let backup_name = format!("{file_name}.bak.{timestamp}");
        let parent = self
            .config_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        Ok(parent.join(backup_name))
    }
}

#[async_trait]
impl ConfigManager for TomlConfigManager {
    async fn load_config(&self, path: &Path) -> Result<Config> {
        ConfigParser::from_manifest_file(path)
            .with_context(|| format!("Failed to parse config: {}", path.display()))
    }

    async fn save_config(&self, _config: &Config, _path: &Path) -> Result<()> {
        Err(anyhow::anyhow!(
            "Saving parsed Config is not supported; update manifest.toml directly"
        ))
    }

    async fn update_dependency(&self, spec: &DependencySpec) -> Result<()> {
        let contents = self.read_config_string(&self.config_path).await?;
        let mut doc = contents
            .parse::<DocumentMut>()
            .with_context(|| format!("Failed to parse config: {}", self.config_path.display()))?;

        if !doc.contains_key("dependencies") {
            doc["dependencies"] = Item::Table(Table::new());
        }

        // Preserve existing dependency entry if it exists
        let existing_dep = doc["dependencies"]
            .get(&spec.alias)
            .and_then(|item| item.as_inline_table());

        let mut dep_table = InlineTable::new();

        // Add name attribute if it differs from alias
        if spec.name != spec.alias {
            dep_table.insert("name", Value::from(spec.name.clone()));
        }

        // Add actr_type attribute - preserve existing if new one is not provided
        if let Some(actr_type) = &spec.actr_type {
            let actr_type_repr = actr_type.to_string_repr();
            if actr_type_repr.is_empty() {
                return Err(anyhow::anyhow!(
                    "Actr type is required for dependency: {}",
                    spec.alias
                ));
            }
            dep_table.insert("actr_type", Value::from(actr_type_repr));
        } else if let Some(existing) = existing_dep {
            // Preserve existing actr_type if new spec doesn't have one
            if let Some(existing_actr_type) = existing.get("actr_type") {
                dep_table.insert("actr_type", existing_actr_type.clone());
            }
        }

        // Add fingerprint - preserve existing if new one is not provided
        if let Some(fingerprint) = &spec.fingerprint {
            dep_table.insert("fingerprint", Value::from(fingerprint.as_str()));
        } else if let Some(existing) = existing_dep {
            // Preserve existing fingerprint if new spec doesn't have one
            if let Some(existing_fp) = existing.get("fingerprint") {
                dep_table.insert("fingerprint", existing_fp.clone());
            }
        }

        doc["dependencies"][&spec.alias] = Item::Value(Value::InlineTable(dep_table));

        self.write_config_string(&self.config_path, &doc.to_string())
            .await
    }

    async fn validate_config(&self) -> Result<ConfigValidation> {
        let mut errors = Vec::new();
        let warnings = Vec::new();

        let config = match ConfigParser::from_manifest_file(&self.config_path) {
            Ok(config) => config,
            Err(e) => {
                errors.push(format!("Failed to parse config: {e}"));
                return Ok(ConfigValidation {
                    is_valid: false,
                    errors,
                    warnings,
                });
            }
        };

        if config.package.name.trim().is_empty() {
            errors.push("package.name is required".to_string());
        }

        for dependency in &config.dependencies {
            if dependency.alias.trim().is_empty() {
                errors.push("dependency alias is required".to_string());
            }
            if let Some(actr_type) = &dependency.actr_type
                && actr_type.name.trim().is_empty()
            {
                errors.push(format!(
                    "dependency {} has an empty actr_type name",
                    dependency.alias
                ));
            }
        }

        Ok(ConfigValidation {
            is_valid: errors.is_empty(),
            errors,
            warnings,
        })
    }

    fn get_project_root(&self) -> &Path {
        &self.project_root
    }

    async fn backup_config(&self) -> Result<ConfigBackup> {
        if !self.config_path.exists() {
            return Err(anyhow::anyhow!(
                "Config file not found: {}",
                self.config_path.display()
            ));
        }

        let backup_path = self.build_backup_path()?;
        fs::copy(&self.config_path, &backup_path)
            .await
            .with_context(|| {
                format!(
                    "Failed to backup config from {} to {}",
                    self.config_path.display(),
                    backup_path.display()
                )
            })?;

        Ok(ConfigBackup {
            original_path: self.config_path.clone(),
            backup_path,
            timestamp: SystemTime::now(),
        })
    }

    async fn restore_backup(&self, backup: ConfigBackup) -> Result<()> {
        fs::copy(&backup.backup_path, &backup.original_path)
            .await
            .with_context(|| {
                format!(
                    "Failed to restore config from {} to {}",
                    backup.backup_path.display(),
                    backup.original_path.display()
                )
            })?;
        Ok(())
    }

    async fn remove_backup(&self, backup: ConfigBackup) -> Result<()> {
        if backup.backup_path.exists() {
            fs::remove_file(&backup.backup_path)
                .await
                .with_context(|| {
                    format!(
                        "Failed to remove backup file: {}",
                        backup.backup_path.display()
                    )
                })?;
        }
        Ok(())
    }
}

fn resolve_project_root(config_path: &Path) -> PathBuf {
    let canonical_path =
        std::fs::canonicalize(config_path).expect("Failed to canonicalize config path");
    canonical_path
        .parent()
        .expect("Config path must have a parent directory")
        .to_path_buf()
}
