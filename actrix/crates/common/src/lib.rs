//! Base 基础设施库
//!
//! 为 Actor-RTC 辅助服务提供基础设施组件，包括身份管理、加密、监控、存储、租户管理等核心功能

pub mod aid;
pub mod error;
pub mod metrics;
pub mod monitoring;
pub mod storage;
pub mod tenant;
pub mod types;

pub mod config;
pub mod util;

// Re-export commonly used types for convenience
pub use aid::{AIdCredential, AIdCredentialValidator, AidError, IdentityClaims};
pub use error::{
    BaseError, ConfigError, DatabaseError, NetworkError, Result, SerializationError, StorageError,
    ValidationError,
};
pub use monitoring::{ServiceCollector, ServiceInfo, ServiceState, ServiceType};
pub use storage::SqliteNonceStorage;
pub use tenant::{ActorAcl, Tenant, TenantError};
pub use types::{ActrId, PeerId, TenantId};
pub use util::TlsConfigurer;

// Simplified credential module for backward compatibility
pub mod token {
    pub use crate::aid::credential::{AIdCredentialValidator, AidError};
}

// Create a status module for backward compatibility
pub mod status {
    pub mod services {
        pub use crate::monitoring::ServiceState;
    }
}
