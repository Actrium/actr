//! Shared user configuration resolver.

use super::loader::{global_config_path, load_cli_config, local_config_path};
use super::schema::{
    CacheConfig, CliConfig, CodegenConfig, InitConfig, InstallConfig, NetworkConfig, StorageConfig,
    UiConfig,
};
use crate::error::Result;
use std::path::PathBuf;

/// Fully-resolved user config with all defaults applied.
#[derive(Debug, Clone)]
pub struct EffectiveCliConfig {
    pub init: EffectiveInitConfig,
    pub codegen: EffectiveCodegenConfig,
    pub cache: EffectiveCacheConfig,
    pub ui: EffectiveUiConfig,
    pub network: EffectiveNetworkConfig,
    pub storage: EffectiveStorageConfig,
}

#[derive(Debug, Clone)]
pub struct EffectiveInitConfig {
    pub manufacturer: String,
}

#[derive(Debug, Clone)]
pub struct EffectiveCodegenConfig {
    pub language: String,
    pub output: String,
    pub clean_before_generate: bool,
}

#[derive(Debug, Clone)]
pub struct EffectiveCacheConfig {
    pub dir: String,
    pub auto_lock: bool,
    pub prefer_cache: bool,
}

#[derive(Debug, Clone)]
pub struct EffectiveUiConfig {
    pub format: String,
    pub verbose: bool,
    pub color: String,
    pub non_interactive: bool,
}

#[derive(Debug, Clone)]
pub struct EffectiveNetworkConfig {
    pub signaling_url: String,
    pub ais_endpoint: String,
    pub realm_id: Option<u32>,
    pub realm_secret: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EffectiveStorageConfig {
    pub hyper_data_dir: PathBuf,
}

impl Default for EffectiveCliConfig {
    fn default() -> Self {
        apply_defaults(CliConfig::default())
    }
}

/// Resolve the effective CLI/user config by merging global and local configs, then applying defaults.
pub fn resolve_effective_cli_config() -> Result<EffectiveCliConfig> {
    let global = load_cli_config(&global_config_path()?)?;
    let local = load_cli_config(&local_config_path())?;
    let merged = merge_configs(global, local);
    Ok(apply_defaults(merged))
}

/// Resolve only the effective Hyper data directory from the shared user config.
pub fn resolve_hyper_data_dir() -> Result<PathBuf> {
    Ok(resolve_effective_cli_config()?.storage.hyper_data_dir)
}

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
            cache: CacheConfig {
                dir: o.cache.dir.or(b.cache.dir),
                auto_lock: o.cache.auto_lock.or(b.cache.auto_lock),
                prefer_cache: o.cache.prefer_cache.or(b.cache.prefer_cache),
            },
            ui: UiConfig {
                format: o.ui.format.or(b.ui.format),
                verbose: o.ui.verbose.or(b.ui.verbose),
                color: o.ui.color.or(b.ui.color),
                non_interactive: o.ui.non_interactive.or(b.ui.non_interactive),
            },
            network: NetworkConfig {
                signaling_url: o.network.signaling_url.or(b.network.signaling_url),
                ais_endpoint: o.network.ais_endpoint.or(b.network.ais_endpoint),
                realm_id: o.network.realm_id.or(b.network.realm_id),
                realm_secret: o.network.realm_secret.or(b.network.realm_secret),
            },
            install: InstallConfig {},
            storage: StorageConfig {
                hyper_data_dir: o.storage.hyper_data_dir.or(b.storage.hyper_data_dir),
            },
        },
    }
}

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
        cache: EffectiveCacheConfig {
            dir: cfg.cache.dir.unwrap_or_else(|| "~/.actr/cache".to_string()),
            auto_lock: cfg.cache.auto_lock.unwrap_or(true),
            prefer_cache: cfg.cache.prefer_cache.unwrap_or(true),
        },
        ui: EffectiveUiConfig {
            format: cfg.ui.format.unwrap_or_else(|| "toml".to_string()),
            verbose: cfg.ui.verbose.unwrap_or(false),
            color: cfg.ui.color.unwrap_or_else(|| "auto".to_string()),
            non_interactive: cfg.ui.non_interactive.unwrap_or(false),
        },
        network: EffectiveNetworkConfig {
            signaling_url: cfg
                .network
                .signaling_url
                .unwrap_or_else(|| "ws://localhost:8081/signaling/ws".to_string()),
            ais_endpoint: cfg
                .network
                .ais_endpoint
                .unwrap_or_else(|| "http://localhost:8081/ais".to_string()),
            realm_id: cfg.network.realm_id.or(Some(1)),
            realm_secret: cfg.network.realm_secret,
        },
        storage: EffectiveStorageConfig {
            hyper_data_dir: cfg
                .storage
                .hyper_data_dir
                .map(expand_tilde)
                .unwrap_or_else(|| expand_tilde("~/.actr/hyper".to_string())),
        },
    }
}

fn expand_tilde(path: String) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_include_global_hyper_dir() {
        let effective = EffectiveCliConfig::default();
        assert_eq!(
            effective.storage.hyper_data_dir,
            expand_tilde("~/.actr/hyper".to_string())
        );
    }

    #[test]
    fn overlay_wins_for_storage() {
        let base = CliConfig {
            storage: StorageConfig {
                hyper_data_dir: Some("/tmp/base".to_string()),
            },
            ..Default::default()
        };
        let overlay = CliConfig {
            storage: StorageConfig {
                hyper_data_dir: Some("/tmp/overlay".to_string()),
            },
            ..Default::default()
        };

        let merged = merge_configs(Some(base), Some(overlay));
        assert_eq!(
            merged.storage.hyper_data_dir.as_deref(),
            Some("/tmp/overlay")
        );
    }
}
