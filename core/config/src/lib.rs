//! # actr-config
//!
//! Configuration parsers for workload manifests and Hyper runtime config.
//!
//! This crate provides a two-layer configuration system:
//! - `ManifestRawConfig` / `ManifestConfig`: Parsed from `manifest.toml` (package metadata)
//! - `RuntimeRawConfig` / `RuntimeConfig`: Parsed from `actr.toml` (deployment config)
//!
//! The parser uses an edition-based system, allowing the configuration format
//! to evolve over time while maintaining backward compatibility.
//!
//! # Example
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use actr_config::ConfigParser;
//!
//! // Parse manifest.toml (workload package metadata)
//! let manifest = ConfigParser::from_manifest_file("manifest.toml")?;
//! println!("Package: {}", manifest.package.name);
//!
//! // Parse actr.toml (runtime deployment config) — requires package info
//! let package = manifest.package.clone();
//! let runtime = ConfigParser::from_runtime_file("actr.toml", package, manifest.tags.clone())?;
//! println!("Signaling: {}", runtime.signaling_url);
//! println!("Realm: {}", runtime.realm.realm_id);
//! # Ok(())
//! # }
//! ```

// Core modules
pub mod actr_raw;
pub mod config;
pub mod error;
pub mod lock;
pub mod parser;
pub mod raw;
pub mod user_config;

// Re-exports for convenience
pub use actr_raw::*;
pub use config::*;
pub use error::*;
pub use lock::*;
pub use parser::*;
pub use raw::*;
pub use user_config::*;

/// Re-export the new config types
pub use config::{ManifestConfig, RuntimeConfig};

/// Re-export commonly used types
pub use serde::{Deserialize, Serialize};
pub use url::Url;
