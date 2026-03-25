//! ACTR-CLI core reuse component module
//!
//! Implements a unified CLI reuse architecture with 8 core components
//! and 3 operation pipelines, providing consistent UX and high code reuse.

pub mod components;
pub mod container;
pub mod error;
pub mod pipelines;

// Re-export core types
pub use components::*;
pub use container::*;
pub use error::*;
pub use pipelines::*;
