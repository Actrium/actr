//! Admin control-plane library for actrix nodes.
//!
//! This crate is the canonical implementation for node-side control-plane
//! behavior (register/report client + node_admin gRPC API server).

pub mod auth;
pub mod client;
pub mod config;
pub mod error;
pub mod metrics;
pub mod nonce_auth;
pub mod realm;
pub mod service;

pub use auth::AuthService;
pub use client::AdminClient;
pub use config::AdminConfig;
pub use error::{AdminError, Result as AdminResult};
pub use realm::{
    REALM_ENABLED_KEY, REALM_USE_SERVERS_KEY, REALM_VERSION_KEY, RealmMetadata,
    get_max_realm_version,
};
pub use service::AdminApiService;

// Re-export commonly used proto types from actrix-proto.
pub use actrix_proto::{
    ConfigType, ControlHealthCheckRequest as HealthCheckRequest,
    ControlHealthCheckResponse as HealthCheckResponse, ControlService, ControlServiceClient,
    ControlServiceServer, CreateRealmRequest, CreateRealmResponse, DeleteRealmRequest,
    DeleteRealmResponse, Directive, DirectiveType, GetConfigRequest, GetConfigResponse,
    GetNodeInfoRequest, GetNodeInfoResponse, GetRealmRequest, GetRealmResponse, ListRealmsRequest,
    ListRealmsResponse, NodeAdminService, NodeAdminServiceClient, NodeAdminServiceServer,
    NonceCredential, RegisterNodeRequest, RegisterNodeResponse, ReportRequest, ReportResponse,
    ResourceType, ServiceAdvertisement, ServiceAdvertisementStatus, ServiceStatus, ShutdownRequest,
    ShutdownResponse, SystemMetrics, UpdateConfigRequest, UpdateConfigResponse, UpdateRealmRequest,
    UpdateRealmResponse,
};
