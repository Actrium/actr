//! Actrix Protocol Buffer Definitions
//!
//! This crate contains all protocol buffer definitions for Actrix services.
//!
//! # Modules
//!
//! - [`supervisor::v1`]: Supervisor service definitions (SupervisorService and SupervisedService)
//! - [`ks::v1`]: Key Server service definitions
//!
//! # Usage
//!
//! ## Direct module access
//!
//! ```ignore
//! use actrix_proto::supervisor::v1::{RegisterNodeRequest, ReportRequest};
//! use actrix_proto::ks::v1::{GenerateKeyRequest, KeyServerClient};
//! ```
//!
//! ## Convenience re-exports
//!
//! Common types are re-exported at the crate root for convenience:
//!
//! ```ignore
//! use actrix_proto::{
//!     NonceCredential, TenantInfo, ResourceType,
//!     SupervisorServiceClient, SupervisedServiceServer,
//! };
//! ```
//!
//! # Design Notes
//!
//! ## Cross-Package References
//!
//! The `ks.v1` package imports `NonceCredential` from `supervisor.v1` for consistent
//! authentication across all services. When using KS gRPC types directly, reference
//! the credential type via `actrix_proto::supervisor::v1::NonceCredential`.

/// Supervisor service protocol definitions.
///
/// Contains both `SupervisorService` (Node → Supervisor) and
/// `SupervisedService` (Supervisor → Node) definitions.
pub mod supervisor {
    pub mod v1 {
        tonic::include_proto!("supervisor.v1");
    }
}

/// Key Server protocol definitions.
///
/// Contains `KeyServer` service for key generation and management.
pub mod ks {
    pub mod v1 {
        tonic::include_proto!("ks.v1");
    }
}

// ============================================================================
// Re-exports: Common Types (from supervisor.v1)
// ============================================================================

pub use supervisor::v1::{
    // Enums
    ConfigType,
    // Shared message types
    Directive,
    DirectiveType,
    // Authentication
    NonceCredential,
    ResourceType,
    ServiceAdvertisement,
    ServiceAdvertisementStatus,
    ServiceStatus,
    SystemMetrics,
    TenantInfo,
};

// ============================================================================
// Re-exports: SupervisorService (Node calls Supervisor)
// ============================================================================

pub use supervisor::v1::{
    // Health check (aliased to avoid collision with ks::v1)
    HealthCheckRequest as SupervisorHealthCheckRequest,
    HealthCheckResponse as SupervisorHealthCheckResponse,
    // Registration
    RegisterNodeRequest,
    RegisterNodeResponse,
    // Reporting
    ReportRequest,
    ReportResponse,
    // Client and server
    supervisor_service_client::SupervisorServiceClient,
    supervisor_service_server::{SupervisorService, SupervisorServiceServer},
};

// ============================================================================
// Re-exports: SupervisedService (Supervisor calls Node)
// ============================================================================

pub use supervisor::v1::{
    // Tenant/Realm management
    CreateTenantRequest,
    CreateTenantResponse,
    DeleteTenantRequest,
    DeleteTenantResponse,
    // Configuration management
    GetConfigRequest,
    GetConfigResponse,
    // Node control
    GetNodeInfoRequest,
    GetNodeInfoResponse,
    GetTenantRequest,
    GetTenantResponse,
    ListTenantsRequest,
    ListTenantsResponse,
    ShutdownRequest,
    ShutdownResponse,
    UpdateConfigRequest,
    UpdateConfigResponse,
    UpdateTenantRequest,
    UpdateTenantResponse,
    // Client and server
    supervised_service_client::SupervisedServiceClient,
    supervised_service_server::{SupervisedService, SupervisedServiceServer},
};

// ============================================================================
// Re-exports: KeyServer Service
// ============================================================================

pub use ks::v1::{
    // Health check (aliased to avoid collision with supervisor::v1)
    HealthCheckRequest as KsHealthCheckRequest,
    HealthCheckResponse as KsHealthCheckResponse,
    // Client and server
    key_server_client::KeyServerClient,
    key_server_server::{KeyServer, KeyServerServer},
};

// Note: KS proto message types (GenerateKeyRequest, etc.) are NOT re-exported
// here because the ks crate defines its own native Rust types with the same
// names for HTTP/JSON API usage. For gRPC usage, access them via:
//   use actrix_proto::ks::v1::{GenerateKeyRequest, ...};
