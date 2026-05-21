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
//!
//! The only external consumer of this library is the sibling binary
//! `protoc-gen-actrframework`, which consumes the re-exported subset below.
//! The `payload_type_extractor` module is an internal helper used by
//! `modern_generator` and is kept crate-private.

pub(crate) mod modern_generator;
pub(crate) mod payload_type_extractor;

pub use modern_generator::{GeneratorRole, ModernGenerator, RemoteServiceInfo};
