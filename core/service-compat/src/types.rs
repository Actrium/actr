//! Core types for actr-version compatibility analysis
//!
//! These are business logic types specific to compatibility analysis,
//! separate from the protocol layer types in actr-protocol.

use serde::{Deserialize, Serialize};

/// Compatibility level between two protocol versions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum CompatibilityLevel {
    /// No changes detected (semantic fingerprints match)
    FullyCompatible = 0,
    /// Changes present but backward compatible
    BackwardCompatible = 1,
    /// Breaking changes detected
    BreakingChanges = 2,
}

/// Simplified proto file representation for convenience
/// (adapts from actr_protocol::service_spec::Protobuf)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtoFile {
    /// File name (e.g., "user.proto")
    pub name: String,
    /// Proto file content
    pub content: String,
    /// Optional file path (for local development)
    pub path: Option<String>,
}
