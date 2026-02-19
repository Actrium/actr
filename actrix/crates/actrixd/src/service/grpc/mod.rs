//! gRPC 服务模块
//!
//! 管理各种 gRPC 服务的实现

pub mod admin_api;
pub mod ks;

pub use admin_api::AdminApiGrpcService;
pub use ks::KsGrpcService;

/// Compatibility alias for historical API name.
///
/// Kept to avoid breaking existing integrations/tests that still import
/// `SupervisordGrpcService` from `actrix::service`.
#[allow(dead_code)]
pub type SupervisordGrpcService = AdminApiGrpcService;
