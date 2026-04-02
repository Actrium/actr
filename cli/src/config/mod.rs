//! CLI configuration re-export module.
//!
//! The source of truth lives in `actr-config::user_config` so native bindings and the CLI
//! resolve the same user-level configuration.

pub mod loader {
    pub use actr_config::user_config::loader::*;
}

pub mod resolver {
    pub use actr_config::user_config::resolver::*;
}

pub mod schema {
    pub use actr_config::user_config::schema::*;
}

pub use resolver::{EffectiveCliConfig, resolve_effective_cli_config};
pub use schema::{
    CacheConfig, CliConfig, CodegenConfig, InitConfig, InstallConfig, NetworkConfig, StorageConfig,
    UiConfig,
};
