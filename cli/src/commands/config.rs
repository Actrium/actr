//! Config command implementation - manage CLI configuration layers.
//!
//! Supported locations:
//! - Global: `~/.actr/config.toml`
//! - Local override: `.actr/config.toml`

use crate::core::{Command, CommandContext, CommandResult, ComponentType};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use clap::{Args, Subcommand};
use owo_colors::OwoColorize;
use std::path::{Path, PathBuf};
use toml::Value;

#[derive(Args, Clone)]
pub struct ConfigCommand {
    /// Read or write the global CLI config (~/.actr/config.toml)
    #[arg(long, conflicts_with = "local")]
    pub global: bool,

    /// Read or write the project-local CLI config (.actr/config.toml)
    #[arg(long, conflicts_with = "global")]
    pub local: bool,

    #[command(subcommand)]
    pub command: ConfigSubcommand,
}

#[derive(Subcommand, Clone)]
pub enum ConfigSubcommand {
    Set {
        key: String,
        value: String,
    },
    Get {
        key: String,
    },
    List,
    Show {
        #[arg(long, default_value = "toml")]
        format: OutputFormat,
    },
    Unset {
        key: String,
    },
    Test,
}

#[derive(Debug, Clone, clap::ValueEnum, Default)]
pub enum OutputFormat {
    #[default]
    Toml,
    Json,
    Yaml,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfigScope {
    Global,
    Local,
    Merged,
}

#[async_trait]
impl Command for ConfigCommand {
    async fn execute(&self, _ctx: &CommandContext) -> Result<CommandResult> {
        match &self.command {
            ConfigSubcommand::Set { key, value } => self.set_config(key, value).await,
            ConfigSubcommand::Get { key } => self.get_config(key).await,
            ConfigSubcommand::List => self.list_config().await,
            ConfigSubcommand::Show { format } => self.show_config(format).await,
            ConfigSubcommand::Unset { key } => self.unset_config(key).await,
            ConfigSubcommand::Test => self.test_config().await,
        }
    }

    fn required_components(&self) -> Vec<ComponentType> {
        vec![]
    }

    fn name(&self) -> &str {
        "config"
    }

    fn description(&self) -> &str {
        "Manage layered CLI configuration (~/.actr/config.toml and .actr/config.toml)"
    }
}

impl ConfigCommand {
    fn read_scope(&self) -> ConfigScope {
        if self.global {
            ConfigScope::Global
        } else if self.local {
            ConfigScope::Local
        } else {
            ConfigScope::Merged
        }
    }

    fn write_scope(&self) -> ConfigScope {
        if self.global {
            ConfigScope::Global
        } else if self.local {
            ConfigScope::Local
        } else if Path::new("manifest.toml").exists() || Path::new(".actr").exists() {
            ConfigScope::Local
        } else {
            ConfigScope::Global
        }
    }

    fn global_config_path() -> Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Unable to determine home directory"))?;
        Ok(home.join(".actr").join("config.toml"))
    }

    fn local_config_path() -> PathBuf {
        PathBuf::from(".actr").join("config.toml")
    }

    fn scope_label(scope: ConfigScope) -> &'static str {
        match scope {
            ConfigScope::Global => "global",
            ConfigScope::Local => "local",
            ConfigScope::Merged => "merged",
        }
    }

    fn scope_path(scope: ConfigScope) -> Result<PathBuf> {
        match scope {
            ConfigScope::Global => Self::global_config_path(),
            ConfigScope::Local => Ok(Self::local_config_path()),
            ConfigScope::Merged => bail!("Merged scope does not map to a single file"),
        }
    }

    fn empty_table() -> Value {
        Value::Table(toml::map::Map::new())
    }

    fn read_value_from_file(path: &Path) -> Result<Value> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let value = content
            .parse::<Value>()
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(value)
    }

    fn read_optional_value(path: &Path) -> Result<Option<Value>> {
        if !path.exists() {
            return Ok(None);
        }
        Self::read_value_from_file(path).map(Some)
    }

    fn load_scope_value(&self, scope: ConfigScope) -> Result<Value> {
        match scope {
            ConfigScope::Global => Ok(Self::read_optional_value(&Self::global_config_path()?)?
                .unwrap_or_else(Self::empty_table)),
            ConfigScope::Local => Ok(Self::read_optional_value(&Self::local_config_path())?
                .unwrap_or_else(Self::empty_table)),
            ConfigScope::Merged => self.load_merged_value(),
        }
    }

    fn load_merged_value(&self) -> Result<Value> {
        let mut merged = Self::read_optional_value(&Self::global_config_path()?)?
            .unwrap_or_else(Self::empty_table);
        if let Some(local) = Self::read_optional_value(&Self::local_config_path())? {
            Self::merge_values(&mut merged, local);
        }
        Ok(merged)
    }

    fn merge_values(base: &mut Value, overlay: Value) {
        match (base, overlay) {
            (Value::Table(base_table), Value::Table(overlay_table)) => {
                for (key, overlay_value) in overlay_table {
                    if let Some(base_value) = base_table.get_mut(&key) {
                        Self::merge_values(base_value, overlay_value);
                    } else {
                        base_table.insert(key, overlay_value);
                    }
                }
            }
            (base_slot, overlay_value) => *base_slot = overlay_value,
        }
    }

    fn ensure_table(value: &mut Value) -> Result<&mut toml::map::Map<String, Value>> {
        if !matches!(value, Value::Table(_)) {
            *value = Self::empty_table();
        }
        match value {
            Value::Table(table) => Ok(table),
            _ => bail!("Configuration root must be a TOML table"),
        }
    }

    fn set_nested_value(value: &mut Value, key: &str, raw_value: &str) -> Result<()> {
        let parsed_value = raw_value
            .parse::<Value>()
            .unwrap_or_else(|_| Value::String(raw_value.to_string()));
        let parts: Vec<&str> = key.split('.').collect();
        if parts.is_empty() {
            bail!("Configuration key cannot be empty");
        }

        let mut current = value;
        for part in &parts[..parts.len() - 1] {
            let table = Self::ensure_table(current)?;
            current = table
                .entry((*part).to_string())
                .or_insert_with(Self::empty_table);
        }

        let table = Self::ensure_table(current)?;
        table.insert(parts[parts.len() - 1].to_string(), parsed_value);
        Ok(())
    }

    fn get_nested_value<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
        let mut current = value;
        for part in key.split('.') {
            current = match current {
                Value::Table(table) => table.get(part)?,
                _ => return None,
            };
        }
        Some(current)
    }

    fn unset_nested_value(value: &mut Value, key: &str) -> bool {
        let parts: Vec<&str> = key.split('.').collect();
        if parts.is_empty() {
            return false;
        }

        let mut current = value;
        for part in &parts[..parts.len() - 1] {
            current = match current {
                Value::Table(table) => match table.get_mut(*part) {
                    Some(next) => next,
                    None => return false,
                },
                _ => return false,
            };
        }

        match current {
            Value::Table(table) => table.remove(parts[parts.len() - 1]).is_some(),
            _ => false,
        }
    }

    fn collect_keys(prefix: Option<&str>, value: &Value, out: &mut Vec<String>) {
        if let Value::Table(table) = value {
            for (key, nested) in table {
                let full_key = match prefix {
                    Some(prefix) => format!("{prefix}.{key}"),
                    None => key.clone(),
                };
                out.push(full_key.clone());
                Self::collect_keys(Some(&full_key), nested, out);
            }
        }
    }

    fn write_scope_value(scope: ConfigScope, value: &Value) -> Result<PathBuf> {
        let path = Self::scope_path(scope)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(value)
            .with_context(|| format!("Failed to serialize {}", path.display()))?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(path)
    }

    async fn set_config(&self, key: &str, raw_value: &str) -> Result<CommandResult> {
        let scope = self.write_scope();
        let mut value = self.load_scope_value(scope)?;
        Self::set_nested_value(&mut value, key, raw_value)?;
        let path = Self::write_scope_value(scope, &value)?;

        Ok(CommandResult::Success(format!(
            "{} Updated {} config: {} = {}\n{}",
            "✅".green(),
            Self::scope_label(scope).cyan(),
            key.yellow(),
            raw_value.green(),
            path.display()
        )))
    }

    async fn get_config(&self, key: &str) -> Result<CommandResult> {
        let scope = self.read_scope();
        let value = self.load_scope_value(scope)?;
        let nested = Self::get_nested_value(&value, key).ok_or_else(|| {
            anyhow::anyhow!(
                "Configuration key '{}' not found in {} scope",
                key,
                Self::scope_label(scope)
            )
        })?;

        let output = if matches!(nested, Value::Table(_) | Value::Array(_)) {
            toml::to_string_pretty(nested)?
        } else {
            nested.to_string()
        };

        Ok(CommandResult::Success(output.trim().to_string()))
    }

    async fn list_config(&self) -> Result<CommandResult> {
        let scope = self.read_scope();
        let value = self.load_scope_value(scope)?;
        let mut keys = Vec::new();
        Self::collect_keys(None, &value, &mut keys);
        keys.sort();
        keys.dedup();

        if keys.is_empty() {
            return Ok(CommandResult::Success(format!(
                "{} No configuration keys in {} scope",
                "📋".yellow(),
                Self::scope_label(scope)
            )));
        }

        Ok(CommandResult::Success(format!(
            "{} {} configuration keys:\n{}",
            "📋".cyan(),
            Self::scope_label(scope),
            keys.join("\n")
        )))
    }

    async fn show_config(&self, format: &OutputFormat) -> Result<CommandResult> {
        let scope = self.read_scope();
        let value = self.load_scope_value(scope)?;
        let output = match format {
            OutputFormat::Toml => toml::to_string_pretty(&value)?,
            OutputFormat::Json => serde_json::to_string_pretty(&value)?,
            OutputFormat::Yaml => serde_yaml::to_string(&value)?,
        };
        Ok(CommandResult::Success(output))
    }

    async fn unset_config(&self, key: &str) -> Result<CommandResult> {
        let scope = self.write_scope();
        let mut value = self.load_scope_value(scope)?;
        if !Self::unset_nested_value(&mut value, key) {
            bail!(
                "Configuration key '{}' not found in {} scope",
                key,
                Self::scope_label(scope)
            );
        }
        let path = Self::write_scope_value(scope, &value)?;
        Ok(CommandResult::Success(format!(
            "{} Removed {} from {} config\n{}",
            "✅".green(),
            key.cyan(),
            Self::scope_label(scope),
            path.display()
        )))
    }

    async fn test_config(&self) -> Result<CommandResult> {
        let scope = self.read_scope();
        let mut lines = Vec::new();
        match scope {
            ConfigScope::Global => {
                let path = Self::global_config_path()?;
                Self::read_optional_value(&path)?;
                lines.push(format!("{} Global config syntax is valid", "✅".green()));
                lines.push(path.display().to_string());
            }
            ConfigScope::Local => {
                let path = Self::local_config_path();
                Self::read_optional_value(&path)?;
                lines.push(format!("{} Local config syntax is valid", "✅".green()));
                lines.push(path.display().to_string());
            }
            ConfigScope::Merged => {
                let global_path = Self::global_config_path()?;
                let local_path = Self::local_config_path();
                Self::read_optional_value(&global_path)?;
                Self::read_optional_value(&local_path)?;
                let merged = self.load_merged_value()?;
                toml::to_string_pretty(&merged)?;
                lines.push(format!("{} Global config parsed", "✅".green()));
                lines.push(format!("{} Local config parsed", "✅".green()));
                lines.push(format!("{} Merged view is valid", "✅".green()));
            }
        }
        Ok(CommandResult::Success(lines.join("\n")))
    }
}
