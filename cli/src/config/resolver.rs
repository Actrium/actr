//! CLI configuration resolver
//!
//! Merges global and local configs and applies built-in defaults to produce
//! a fully-resolved `EffectiveCliConfig` with no optional fields.

use super::loader::{global_config_path, load_cli_config, local_config_path};
use super::schema::{CacheConfig, CliConfig, CodegenConfig, InitConfig, InstallConfig, UiConfig};
use anyhow::Result;

/// Fully-resolved CLI configuration with all defaults applied.
///
/// No optional fields — every value has been resolved from one of:
/// 1. Local .actr/config.toml override
/// 2. Global ~/.actr/config.toml override
/// 3. Binary built-in defaults
#[derive(Debug, Clone)]
pub struct EffectiveCliConfig {
    pub init: EffectiveInitConfig,
    pub codegen: EffectiveCodegenConfig,
    pub install: EffectiveInstallConfig,
    pub cache: EffectiveCacheConfig,
    pub ui: EffectiveUiConfig,
}

/// Resolved init settings
#[derive(Debug, Clone)]
pub struct EffectiveInitConfig {
    pub manufacturer: String,
}

/// Resolved codegen settings
#[derive(Debug, Clone)]
pub struct EffectiveCodegenConfig {
    pub language: String,
    pub output: String,
    pub clean_before_generate: bool,
}

/// Resolved install settings
#[derive(Debug, Clone)]
pub struct EffectiveInstallConfig {
    pub auto_lock: bool,
    pub prefer_cache: bool,
}

/// Resolved cache settings
#[derive(Debug, Clone)]
pub struct EffectiveCacheConfig {
    pub dir: String,
}

/// Resolved UI/output settings
#[derive(Debug, Clone)]
pub struct EffectiveUiConfig {
    pub format: String,
    pub verbose: bool,
    pub color: String,
    pub non_interactive: bool,
}

impl Default for EffectiveCliConfig {
    fn default() -> Self {
        apply_defaults(CliConfig::default())
    }
}

/// Resolve the effective CLI config by merging global and local configs, then applying defaults.
///
/// Priority (high → low):
/// 1. Local .actr/config.toml
/// 2. Global ~/.actr/config.toml
/// 3. Binary built-in defaults
pub fn resolve_effective_cli_config() -> Result<EffectiveCliConfig> {
    let global = load_cli_config(&global_config_path()?)?;
    let local = load_cli_config(&local_config_path())?;
    let merged = merge_configs(global, local);
    Ok(apply_defaults(merged))
}

/// Merge two optional configs: overlay fields take priority over base fields.
fn merge_configs(base: Option<CliConfig>, overlay: Option<CliConfig>) -> CliConfig {
    match (base, overlay) {
        (None, None) => CliConfig::default(),
        (Some(b), None) => b,
        (None, Some(o)) => o,
        (Some(b), Some(o)) => CliConfig {
            version: o.version.or(b.version),
            init: InitConfig {
                manufacturer: o.init.manufacturer.or(b.init.manufacturer),
            },
            codegen: CodegenConfig {
                language: o.codegen.language.or(b.codegen.language),
                output: o.codegen.output.or(b.codegen.output),
                clean_before_generate: o
                    .codegen
                    .clean_before_generate
                    .or(b.codegen.clean_before_generate),
            },
            install: InstallConfig {
                auto_lock: o.install.auto_lock.or(b.install.auto_lock),
                prefer_cache: o.install.prefer_cache.or(b.install.prefer_cache),
            },
            cache: CacheConfig {
                dir: o.cache.dir.or(b.cache.dir),
            },
            ui: UiConfig {
                format: o.ui.format.or(b.ui.format),
                verbose: o.ui.verbose.or(b.ui.verbose),
                color: o.ui.color.or(b.ui.color),
                non_interactive: o.ui.non_interactive.or(b.ui.non_interactive),
            },
        },
    }
}

/// Apply built-in defaults to produce an `EffectiveCliConfig`.
fn apply_defaults(cfg: CliConfig) -> EffectiveCliConfig {
    EffectiveCliConfig {
        init: EffectiveInitConfig {
            manufacturer: cfg.init.manufacturer.unwrap_or_else(|| "acme".to_string()),
        },
        codegen: EffectiveCodegenConfig {
            language: cfg.codegen.language.unwrap_or_else(|| "rust".to_string()),
            output: cfg
                .codegen
                .output
                .unwrap_or_else(|| "src/generated".to_string()),
            clean_before_generate: cfg.codegen.clean_before_generate.unwrap_or(false),
        },
        install: EffectiveInstallConfig {
            auto_lock: cfg.install.auto_lock.unwrap_or(true),
            prefer_cache: cfg.install.prefer_cache.unwrap_or(true),
        },
        cache: EffectiveCacheConfig {
            dir: cfg
                .cache
                .dir
                .unwrap_or_else(|| "~/.actr/cache".to_string()),
        },
        ui: EffectiveUiConfig {
            format: cfg.ui.format.unwrap_or_else(|| "toml".to_string()),
            verbose: cfg.ui.verbose.unwrap_or(false),
            color: cfg.ui.color.unwrap_or_else(|| "auto".to_string()),
            non_interactive: cfg.ui.non_interactive.unwrap_or(false),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_defaults() {
        let config = CliConfig::default();
        let effective = apply_defaults(config);
        assert_eq!(effective.init.manufacturer, "acme");
        assert_eq!(effective.codegen.language, "rust");
        assert_eq!(effective.codegen.output, "src/generated");
        assert!(!effective.codegen.clean_before_generate);
        assert!(effective.install.auto_lock);
        assert!(effective.install.prefer_cache);
        assert_eq!(effective.cache.dir, "~/.actr/cache");
        assert_eq!(effective.ui.format, "toml");
        assert!(!effective.ui.verbose);
        assert_eq!(effective.ui.color, "auto");
        assert!(!effective.ui.non_interactive);
    }

    #[test]
    fn test_merge_configs_none_none() {
        let merged = merge_configs(None, None);
        assert!(merged.init.manufacturer.is_none());
    }

    #[test]
    fn test_merge_configs_overlay_wins() {
        let base = CliConfig {
            init: super::super::schema::InitConfig {
                manufacturer: Some("base-org".to_string()),
            },
            ..Default::default()
        };
        let overlay = CliConfig {
            init: super::super::schema::InitConfig {
                manufacturer: Some("overlay-org".to_string()),
            },
            ..Default::default()
        };
        let merged = merge_configs(Some(base), Some(overlay));
        assert_eq!(merged.init.manufacturer.as_deref(), Some("overlay-org"));
    }

    #[test]
    fn test_merge_configs_base_fallback() {
        let base = CliConfig {
            init: super::super::schema::InitConfig {
                manufacturer: Some("base-org".to_string()),
            },
            ..Default::default()
        };
        let merged = merge_configs(Some(base), None);
        assert_eq!(merged.init.manufacturer.as_deref(), Some("base-org"));
    }

    #[test]
    fn test_effective_cli_config_default() {
        let effective = EffectiveCliConfig::default();
        assert_eq!(effective.init.manufacturer, "acme");
    }
}
