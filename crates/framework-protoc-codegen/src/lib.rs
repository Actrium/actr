//! # actr-framework-protoc-codegen
//!
//! Protoc plugin for generating actr-framework code from protobuf service definitions.
//!
//! This crate generates:
//! - Handler traits for service implementations
//! - MessageDispatcher implementations for request routing
//! - Workload wrapper types
//! - Message trait implementations (for Context::call/tell)
//! - PayloadType-aware client code

pub mod modern_generator;
pub mod payload_type_extractor;

pub use modern_generator::{GeneratorRole, ModernGenerator};
pub use payload_type_extractor::{
    PayloadType, extract_payload_type, extract_payload_type_or_default,
};
