//! # actr-config
//!
//! Configuration parsers for workload manifests and Hyper runtime config.
//!
//! This crate provides a two-layer configuration system:
//! - `ManifestRawConfig`: Direct TOML mapping for `manifest.toml`
//! - `RuntimeRawConfig`: Direct TOML mapping for `actr.toml`
//! - `Config`: Fully parsed and validated final configuration
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
//! // Parse configuration from file
//! let config = ConfigParser::from_manifest_file("manifest.toml")?;
//!
//! // Access parsed values
//! println!("Package: {}", config.package.name);
//! // Realm comes from actr.toml, not manifest.toml
//! if let Some(realm) = &config.realm {
//!     println!("Realm: {}", realm.realm_id);
//! }
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

// Re-exports for convenience
pub use actr_raw::*;
pub use config::*;
pub use error::*;
pub use lock::*;
pub use parser::*;
pub use raw::*;

/// Re-export commonly used types
pub use serde::{Deserialize, Serialize};
pub use url::Url;
