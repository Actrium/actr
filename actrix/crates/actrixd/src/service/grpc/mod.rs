//! gRPC 服务模块
//!
//! 管理各种 gRPC 服务的实现

pub mod signer;

pub use signer::build_signer_grpc_router;
