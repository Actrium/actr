//! Supervit - gRPC Supervisor communication library for actrix nodes
//!
//! This crate provides both client and server components for
//! gRPC communication between actrix nodes and the actrix-supervisor platform.
//!
//! # Components
//!
//! - **SupervisorService Client**: For nodes to call the supervisor
//!   - Node registration
//!   - Status reporting (unary RPC)
//!   - Health checks
//!
//! - **SupervisedService Server**: For supervisor to call nodes
//!   - Configuration management
//!   - Realm (tenant) CRUD operations
//!   - Node control (info, shutdown)
//!
//! # Architecture
//!
//! ```text
//!     ┌─────────────────────┐                    ┌─────────────────────┐
//!     │    actrix-node      │                    │  actrix-supervisor  │
//!     ├─────────────────────┤                    ├─────────────────────┤
//!     │                     │   Node → Super     │                     │
//!     │  SupervisorService  │ ─────────────────► │  SupervisorService  │
//!     │  Client             │                    │  Server             │
//!     │                     │                    │                     │
//!     │  SupervisedService  │   Super → Node     │  SupervisedService  │
//!     │  Server             │ ◄───────────────── │  Client             │
//!     └─────────────────────┘                    └─────────────────────┘
//! ```
//!
//! # Naming
//!
//! The name "supervit" combines "Supervisor" + "it", representing the
//! bidirectional communication components:
//! - SupervisorService client (to call Supervisor)
//! - SupervisedService server (to be called by Supervisor)

pub mod auth;
pub mod client;
pub mod config;
pub mod error;
pub mod metrics;
pub mod nonce_auth;
pub mod realm;
pub mod service;

// Re-export important types and functions
pub use auth::AuthService;
pub use client::SupervitClient;
pub use config::SupervitConfig;
pub use error::{Result, SupervitError};
pub use realm::{
    REALM_ENABLED_KEY, REALM_USE_SERVERS_KEY, REALM_VERSION_KEY, RealmMetadata,
    get_max_realm_version,
};
pub use service::Supervisord;

// Re-export commonly used proto types from actrix-proto
pub use actrix_proto::{
    // Common types
    ConfigType,
    // SupervisedService (Supervisor calls Node)
    CreateTenantRequest,
    CreateTenantResponse,
    DeleteTenantRequest,
    DeleteTenantResponse,
    Directive,
    DirectiveType,
    GetConfigRequest,
    GetConfigResponse,
    GetNodeInfoRequest,
    GetNodeInfoResponse,
    GetTenantRequest,
    GetTenantResponse,
    ListTenantsRequest,
    ListTenantsResponse,
    NonceCredential,
    RegisterNodeRequest,
    RegisterNodeResponse,
    ReportRequest,
    ReportResponse,
    ResourceType,
    ServiceAdvertisement,
    ServiceAdvertisementStatus,
    ServiceStatus,
    ShutdownRequest,
    ShutdownResponse,
    SupervisedService,
    SupervisedServiceClient,
    SupervisedServiceServer,
    // SupervisorService (Node calls Supervisor)
    SupervisorHealthCheckRequest as HealthCheckRequest,
    SupervisorHealthCheckResponse as HealthCheckResponse,
    SupervisorService,
    SupervisorServiceClient,
    SupervisorServiceServer,
    SystemMetrics,
    TenantInfo,
    UpdateConfigRequest,
    UpdateConfigResponse,
    UpdateTenantRequest,
    UpdateTenantResponse,
};

/// Realm info type alias to reduce tenant wording in the code.
pub type RealmInfo = TenantInfo;
