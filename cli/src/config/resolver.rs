//! CLI configuration resolver
//!
//! Merges global and local configs and applies built-in defaults to produce
//! a fully-resolved `EffectiveCliConfig` with no optional fields.

use super::loader::{global_config_path, load_cli_config, local_config_path};
use super::schema::{
    CacheConfig, CliConfig, CodegenConfig, InstallConfig, MfrConfig, NetworkConfig, StorageConfig,
    UiConfig,
};
use anyhow::Result;
use std::path::PathBuf;

/// Fully-resolved CLI configuration with all defaults applied.
///
/// No optional fields — every value has been resolved from one of:
/// 1. Local .actr/config.toml override
/// 2. Global ~/.actr/config.toml override
/// 3. Binary built-in defaults
#[derive(Debug, Clone)]
pub struct EffectiveCliConfig {
    pub mfr: EffectiveMfrConfig,
    pub codegen: EffectiveCodegenConfig,
    pub cache: EffectiveCacheConfig,
    pub ui: EffectiveUiConfig,
    pub network: EffectiveNetworkConfig,
    pub storage: EffectiveStorageConfig,
}

/// Resolved manufacturer identity settings
#[derive(Debug, Clone)]
pub struct EffectiveMfrConfig {
    pub manufacturer: String,
    pub keychain: Option<String>,
}

/// Resolved codegen settings
#[derive(Debug, Clone)]
pub struct EffectiveCodegenConfig {
    pub language: String,
    pub output: String,
    pub clean_before_generate: bool,
}

/// Resolved cache settings
#[derive(Debug, Clone)]
pub struct EffectiveCacheConfig {
    pub dir: String,
    pub auto_lock: bool,
    pub prefer_cache: bool,
}

/// Resolved UI/output settings
#[derive(Debug, Clone)]
pub struct EffectiveUiConfig {
    pub format: String,
    pub verbose: bool,
    pub color: String,
    pub non_interactive: bool,
}

/// Resolved network settings
///
/// Used by CLI network operations (check/install/discovery).
/// Note: realm_id defaults to 1 if not explicitly configured.
#[derive(Debug, Clone)]
pub struct EffectiveNetworkConfig {
    /// Signaling server URL (default: ws://localhost:8081/signaling/ws)
    pub signaling_url: String,

    /// AIS endpoint (default: http://localhost:8081/ais)
    pub ais_endpoint: String,

    /// Realm ID (default: 1)
    pub realm_id: Option<u32>,

    /// Realm secret (optional)
    pub realm_secret: Option<String>,
}

/// Resolved storage settings
#[derive(Debug, Clone)]
pub struct EffectiveStorageConfig {
    pub hyper_data_dir: PathBuf,
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
            mfr: MfrConfig {
                manufacturer: o.mfr.manufacturer.or(b.mfr.manufacturer),
                keychain: o.mfr.keychain.or(b.mfr.keychain),
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

/// Apply built-in defaults to produce an `EffectiveCliConfig`.
fn apply_defaults(cfg: CliConfig) -> EffectiveCliConfig {
    EffectiveCliConfig {
        mfr: EffectiveMfrConfig {
            manufacturer: cfg.mfr.manufacturer.unwrap_or_else(|| "acme".to_string()),
            keychain: cfg
                .mfr
                .keychain
                .map(|p| expand_tilde(p).to_string_lossy().to_string()),
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
    fn test_apply_defaults() {
        let config = CliConfig::default();
        let effective = apply_defaults(config);
        assert_eq!(effective.mfr.manufacturer, "acme");
        assert!(effective.mfr.keychain.is_none());
        assert_eq!(effective.codegen.language, "rust");
        assert_eq!(effective.codegen.output, "src/generated");
        assert!(!effective.codegen.clean_before_generate);
        assert_eq!(effective.cache.dir, "~/.actr/cache");
        assert!(effective.cache.auto_lock);
        assert!(effective.cache.prefer_cache);
        assert_eq!(effective.ui.format, "toml");
        assert!(!effective.ui.verbose);
        assert_eq!(effective.ui.color, "auto");
        assert!(!effective.ui.non_interactive);
        assert_eq!(
            effective.storage.hyper_data_dir,
            expand_tilde("~/.actr/hyper".to_string())
        );
    }

    #[test]
    fn test_merge_configs_none_none() {
        let merged = merge_configs(None, None);
        assert!(merged.mfr.manufacturer.is_none());
        assert!(merged.mfr.keychain.is_none());
    }

    #[test]
    fn test_merge_configs_overlay_wins() {
        let base = CliConfig {
            mfr: MfrConfig {
                manufacturer: Some("base-org".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let overlay = CliConfig {
            mfr: MfrConfig {
                manufacturer: Some("overlay-org".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_configs(Some(base), Some(overlay));
        assert_eq!(merged.mfr.manufacturer.as_deref(), Some("overlay-org"));
    }

    #[test]
    fn test_merge_configs_base_fallback() {
        let base = CliConfig {
            mfr: MfrConfig {
                manufacturer: Some("base-org".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_configs(Some(base), None);
        assert_eq!(merged.mfr.manufacturer.as_deref(), Some("base-org"));
    }

    #[test]
    fn test_effective_cli_config_default() {
        let effective = EffectiveCliConfig::default();
        assert_eq!(effective.mfr.manufacturer, "acme");
    }

    #[test]
    fn test_network_defaults() {
        let config = CliConfig::default();
        let effective = apply_defaults(config);
        assert_eq!(
            effective.network.signaling_url,
            "ws://localhost:8081/signaling/ws"
        );
        assert_eq!(effective.network.ais_endpoint, "http://localhost:8081/ais");
        assert_eq!(effective.network.realm_id, Some(1));
        assert!(effective.network.realm_secret.is_none());
    }

    #[test]
    fn test_network_merge_overlay_wins() {
        let base = CliConfig {
            network: NetworkConfig {
                signaling_url: Some("ws://base:8081/signaling/ws".to_string()),
                realm_id: Some(1000),
                ..Default::default()
            },
            ..Default::default()
        };
        let overlay = CliConfig {
            network: NetworkConfig {
                signaling_url: Some("ws://overlay:8081/signaling/ws".to_string()),
                realm_id: Some(2000),
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_configs(Some(base), Some(overlay));
        assert_eq!(
            merged.network.signaling_url.as_deref(),
            Some("ws://overlay:8081/signaling/ws")
        );
        assert_eq!(merged.network.realm_id, Some(2000));
    }

    #[test]
    fn test_network_partial_override() {
        let base = CliConfig {
            network: NetworkConfig {
                signaling_url: Some("ws://base:8081/signaling/ws".to_string()),
                realm_id: Some(1000),
                ..Default::default()
            },
            ..Default::default()
        };
        let overlay = CliConfig {
            network: NetworkConfig {
                realm_id: Some(2000),
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_configs(Some(base), Some(overlay));
        // signaling_url from base
        assert_eq!(
            merged.network.signaling_url.as_deref(),
            Some("ws://base:8081/signaling/ws")
        );
        // realm_id from overlay
        assert_eq!(merged.network.realm_id, Some(2000));
    }
}
