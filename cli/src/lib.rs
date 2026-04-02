//! ACTR-CLI Library
//!
//! Provides core functionality modules for the Actor-RTC CLI tool

pub mod assets;
pub mod commands;
pub mod config;
pub mod core;
pub mod error;
pub mod plugin_config;
pub mod project_language;
pub mod templates;
#[cfg(feature = "test-utils")]
pub mod test_support;
pub mod web_assets;
pub use templates as template;
pub mod utils;

// Re-export commonly used types
pub use core::*;
