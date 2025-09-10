//! # actr-config
//! 
//! Configuration file parser and project manifest support for Actor-RTC framework.
//! 
//! This crate provides support for parsing and managing `actr.toml` project manifest files,
//! which contain project metadata, proto dependencies, and routing rules.

pub mod config;
pub mod error;
pub mod dependencies;
pub mod routing;

pub use config::*;
pub use error::*;
pub use dependencies::*;
pub use routing::*;

/// Re-export commonly used types
pub use serde::{Deserialize, Serialize};
pub use url::Url;