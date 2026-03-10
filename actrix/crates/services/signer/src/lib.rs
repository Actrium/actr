//! Signer - Ed25519 签名密钥生成和管理服务
//!
//! Signer 服务提供以下功能：
//! 1. 生成 Ed25519 签名密钥对，私钥永不离开 Signer
//! 2. 对外提供 Sign API，代替调用方执行签名操作
//! 3. PSK 签名验证和防重放攻击保护
//! 4. 多存储后端支持：SQLite, PostgreSQL
#![deny(clippy::disallowed_macros)]

#[cfg(test)]
pub mod client;
pub mod config;
pub mod crypto;
pub mod error;
pub mod grpc_client;
pub mod grpc_handlers;
pub mod handlers;
pub mod recording;
pub mod storage;
pub mod types;

// Re-export commonly used items
#[cfg(test)]
pub use client::{Client, ClientConfig};
pub use config::SignerServiceConfig;
pub use crypto::{KekSource, KeyEncryptor};
pub use error::SignerError;
pub use grpc_client::{GrpcClient, GrpcClientConfig};
pub use grpc_handlers::{SignerGrpcService, create_grpc_service};
// Re-export proto types from actrix-proto
pub use actrix_proto::signer::v1::signer_server::{Signer, SignerServer};
pub use handlers::{
    SignerState, create_router, create_signer_state, get_stats, register_signer_metrics,
};
pub use storage::{KeyStorage, StorageConfig};
pub use types::{
    GenerateSigningKeyRequest, GenerateSigningKeyResponse, KeyPair, KeyRecord, SignRequest,
    SignResponse,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{SqliteConfig, StorageBackend, StorageConfig};
    use nonce_auth::storage::MemoryStorage;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_signer_service_creation() {
        let temp_dir = tempdir().unwrap();

        let config = SignerServiceConfig {
            storage: StorageConfig {
                backend: StorageBackend::Sqlite,
                key_ttl_seconds: 3600,
                sqlite: Some(SqliteConfig {}),
                postgres: None,
            },
            kek: None,
            kek_env: None,
            kek_file: None,
            tolerance_seconds: 3600,
        };

        // 使用内存存储进行测试（避免文件系统依赖）
        let nonce_storage = MemoryStorage::new();
        let state = create_signer_state(
            &config,
            nonce_storage,
            "test-actrix-shared-key",
            temp_dir.path(),
        )
        .await;
        assert!(state.is_ok());
    }
}
