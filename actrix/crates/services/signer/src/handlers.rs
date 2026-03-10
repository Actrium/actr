//! Signer HTTP 处理器

use crate::{
    crypto::KeyEncryptor,
    error::SignerError,
    storage::KeyStorage,
    types::{GenerateSigningKeyRequest, GenerateSigningKeyResponse, SignRequest, SignResponse},
};
use axum::{
    Router,
    extract::{Json, Path, State},
    routing::{get, post},
};
use lazy_static::lazy_static;
use nonce_auth::{CredentialVerifier, NonceError, storage::NonceStorage};
use prometheus::{HistogramOpts, HistogramVec, IntCounterVec, Opts};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use std::time::Instant;

lazy_static! {
    /// Signer 服务指标
    static ref KS_KEYS_GENERATED: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_keys_generated_total", "Total number of keys generated")
            .namespace("actrix"),
        &["key_type"]
    ).unwrap();

    static ref KS_REQUEST_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new("actrix_request_duration_seconds", "HTTP request duration in seconds")
            .namespace("actrix")
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]),
        &["service", "method", "path", "status"]
    ).unwrap();

    static ref KS_REQUESTS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_requests_total", "Total number of HTTP requests")
            .namespace("actrix"),
        &["service", "method", "path", "status"]
    ).unwrap();

    static ref KS_AUTH_FAILURES: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_auth_failures_total", "Total number of authentication failures")
            .namespace("actrix"),
        &["service", "reason"]
    ).unwrap();
}

/// 注册 Signer metrics 到全局 registry
pub fn register_signer_metrics(registry: &prometheus::Registry) -> Result<(), prometheus::Error> {
    registry.register(Box::new(KS_KEYS_GENERATED.clone()))?;
    registry.register(Box::new(KS_REQUEST_DURATION.clone()))?;
    registry.register(Box::new(KS_REQUESTS_TOTAL.clone()))?;
    registry.register(Box::new(KS_AUTH_FAILURES.clone()))?;
    Ok(())
}

/// 惰性清理触发条件
const CLEANUP_CHECK_INTERVAL: u32 = 100; // 每 100 次请求检查一次
const CLEANUP_MIN_KEYS: u32 = 10; // 至少有 10 个密钥时才清理

/// Signer 服务状态
#[derive(Clone)]
pub struct SignerState {
    pub storage: KeyStorage,
    pub nonce_storage: Arc<dyn NonceStorage + Send + Sync>,
    pub psk: String,
    /// 容忍期（秒）
    pub tolerance_seconds: u64,
    /// 请求计数器（用于惰性清理触发）
    request_counter: Arc<AtomicU32>,
}

impl SignerState {
    pub fn new<N: NonceStorage + Send + Sync + 'static>(
        storage: KeyStorage,
        nonce_storage: N,
        psk: String,
        tolerance_seconds: u64,
    ) -> Self {
        Self {
            storage,
            nonce_storage: Arc::new(nonce_storage),
            psk,
            tolerance_seconds,
            request_counter: Arc::new(AtomicU32::new(0)),
        }
    }

    /// 惰性清理：在请求时检查是否需要清理过期密钥
    ///
    /// 触发条件：
    /// - 每 CLEANUP_CHECK_INTERVAL 次请求检查一次
    /// - 数据库中至少有 CLEANUP_MIN_KEYS 个密钥
    async fn maybe_cleanup_expired_keys(&self) {
        let count = self.request_counter.fetch_add(1, Ordering::Relaxed);

        // 每 N 次请求检查一次
        if count % CLEANUP_CHECK_INTERVAL != 0 {
            return;
        }

        // 在后台异步清理，不阻塞当前请求
        let storage = self.storage.clone();
        let tolerance = self.tolerance_seconds;
        tokio::spawn(async move {
            // 先检查密钥总数
            let total_keys = match storage.get_key_count().await {
                Ok(count) => count,
                Err(e) => {
                    crate::recording::warn!("Failed to get key count for cleanup check: {}", e);
                    return;
                }
            };

            if total_keys < CLEANUP_MIN_KEYS {
                crate::recording::debug!(
                    "Skipping cleanup: only {} keys (threshold: {})",
                    total_keys,
                    CLEANUP_MIN_KEYS
                );
                return;
            }

            // 执行清理（仅删除超出容忍期的密钥）
            match storage.cleanup_expired_keys(tolerance).await {
                Ok(cleaned) => {
                    if cleaned > 0 {
                        crate::recording::info!(
                            "Lazy cleanup: removed {} expired keys (total: {})",
                            cleaned,
                            total_keys
                        );
                    }
                }
                Err(e) => {
                    crate::recording::warn!("Failed to cleanup expired keys: {}", e);
                }
            }
        });
    }

    pub async fn verify_credential(
        &self,
        credential: &nonce_auth::NonceCredential,
        request_payload: &str,
    ) -> Result<(), SignerError> {
        let verify_result = CredentialVerifier::new(self.nonce_storage.clone())
            .with_secret(self.psk.as_bytes())
            .verify(credential, request_payload.as_bytes())
            .await;

        verify_result.map_err(|e| match e {
            NonceError::DuplicateNonce => {
                SignerError::ReplayAttack("Nonce already used".to_string())
            }
            NonceError::TimestampOutOfWindow => {
                SignerError::Authentication("Request timestamp out of range".to_string())
            }
            NonceError::InvalidSignature => {
                SignerError::Authentication("Invalid signature".to_string())
            }
            _ => SignerError::Internal(format!("Authentication error: {e}")),
        })?;

        Ok(())
    }
}

/// 从 KS 配置创建 SignerState
///
/// Note: Nonce storage must be provided by the caller to avoid circular dependencies
/// between ks and platform crates.
pub async fn create_signer_state<N: NonceStorage + Send + Sync + 'static>(
    service_config: &crate::config::SignerServiceConfig,
    nonce_storage: N,
    actrix_shared_key: &str,
    sqlite_path: &std::path::Path,
) -> Result<SignerState, SignerError> {
    crate::recording::info!("Initializing Signer state from SignerServiceConfig");

    // 创建密钥加密器
    let encryptor = match service_config.get_kek_source() {
        Some(kek_source) => {
            crate::recording::info!("KEK configured, enabling private key encryption");
            KeyEncryptor::from_kek_source(&kek_source)?
        }
        None => {
            crate::recording::info!("No KEK configured, private keys will be stored in plaintext");
            KeyEncryptor::no_encryption()
        }
    };

    // 从配置创建存储实例（异步）
    let key_storage =
        KeyStorage::from_config(&service_config.storage, encryptor, sqlite_path).await?;

    Ok(SignerState::new(
        key_storage,
        nonce_storage,
        actrix_shared_key.to_string(),
        service_config.tolerance_seconds,
    ))
}

/// 创建 KS 服务的路由
pub fn create_router(state: SignerState) -> Router {
    Router::new()
        .route("/generate-signing-key", post(generate_signing_key_handler))
        .route("/sign/{key_id}", post(sign_handler))
        .route("/health", get(health_check_handler))
        .with_state(state)
}

/// 获取服务统计信息
pub async fn get_stats(state: &SignerState) -> Result<ServiceStats, SignerError> {
    let key_count = state.storage.get_key_count().await?;
    Ok(ServiceStats { key_count })
}

/// 服务统计信息
#[derive(Debug, Clone)]
pub struct ServiceStats {
    pub key_count: u32,
}

async fn generate_signing_key_handler(
    State(app_state): State<SignerState>,
    Json(request): Json<GenerateSigningKeyRequest>,
) -> Result<Json<GenerateSigningKeyResponse>, SignerError> {
    let start_time = Instant::now();
    crate::recording::info!("Received Ed25519 signing key generation request");

    // 验证凭据
    let request_data = request.request_payload();
    let verify_result = app_state
        .verify_credential(&request.credential, &request_data)
        .await;

    if let Err(ref e) = verify_result {
        // 记录认证失败指标
        let reason = match e {
            SignerError::ReplayAttack(_) => "replay_attack",
            SignerError::Authentication(_) => "invalid_signature",
            _ => "unknown",
        };
        KS_AUTH_FAILURES
            .with_label_values(&["signer", reason])
            .inc();

        let duration = start_time.elapsed().as_secs_f64();
        KS_REQUEST_DURATION
            .with_label_values(&["signer", "POST", "/generate-signing-key", "401"])
            .observe(duration);
        KS_REQUESTS_TOTAL
            .with_label_values(&["signer", "POST", "/generate-signing-key", "401"])
            .inc();

        return verify_result.map(|_| unreachable!());
    }
    verify_result?;

    // 生成并存储 Ed25519 密钥对
    let key_pair = app_state.storage.generate_and_store_key().await?;

    // 获取密钥记录以获取正确的过期时间
    let key_record = app_state
        .storage
        .get_key_record(key_pair.key_id)
        .await?
        .ok_or_else(|| SignerError::Internal("Failed to get key record after creation".into()))?;

    // 惰性清理
    app_state.maybe_cleanup_expired_keys().await;

    // 记录密钥生成指标
    KS_KEYS_GENERATED.with_label_values(&["ed25519"]).inc();

    let response = GenerateSigningKeyResponse {
        key_id: key_pair.key_id,
        verifying_key: key_pair.verifying_key,
        expires_at: key_record.expires_at,
        tolerance_seconds: app_state.tolerance_seconds,
    };

    let duration = start_time.elapsed().as_secs_f64();
    KS_REQUEST_DURATION
        .with_label_values(&["signer", "POST", "/generate-signing-key", "200"])
        .observe(duration);
    KS_REQUESTS_TOTAL
        .with_label_values(&["signer", "POST", "/generate-signing-key", "200"])
        .inc();

    crate::recording::info!(
        "Generated Ed25519 signing key with key_id: {}",
        key_pair.key_id
    );
    Ok(Json(response))
}

async fn sign_handler(
    State(app_state): State<SignerState>,
    Path(key_id): Path<u32>,
    Json(request): Json<SignRequest>,
) -> Result<Json<SignResponse>, SignerError> {
    let start_time = Instant::now();
    crate::recording::info!("Received sign request for key_id: {}", key_id);

    // 验证凭据
    let request_data = request.request_payload();
    let verify_result = app_state
        .verify_credential(&request.credential, &request_data)
        .await;

    if let Err(ref e) = verify_result {
        let reason = match e {
            SignerError::ReplayAttack(_) => "replay_attack",
            SignerError::Authentication(_) => "invalid_signature",
            _ => "unknown",
        };
        KS_AUTH_FAILURES
            .with_label_values(&["signer", reason])
            .inc();

        let duration = start_time.elapsed().as_secs_f64();
        KS_REQUEST_DURATION
            .with_label_values(&["signer", "POST", "/sign", "401"])
            .observe(duration);
        KS_REQUESTS_TOTAL
            .with_label_values(&["signer", "POST", "/sign", "401"])
            .inc();

        return verify_result.map(|_| unreachable!());
    }
    verify_result?;

    // 检查密钥是否在容忍期内有效
    let key_record = app_state
        .storage
        .get_key_record(key_id)
        .await?
        .ok_or(SignerError::KeyNotFound(key_id))?;

    if key_record.expires_at > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if key_record.expires_at + app_state.tolerance_seconds < now {
            crate::recording::warn!("Key {} has expired beyond tolerance period", key_id);
            let duration = start_time.elapsed().as_secs_f64();
            KS_REQUEST_DURATION
                .with_label_values(&["signer", "POST", "/sign", "404"])
                .observe(duration);
            KS_REQUESTS_TOTAL
                .with_label_values(&["signer", "POST", "/sign", "404"])
                .inc();
            return Err(SignerError::KeyNotFound(key_id));
        }
    }

    // 在后端内部签名，私钥不离开 KS
    let signature = app_state
        .storage
        .sign(key_id, &request.message)
        .await
        .map_err(|e| match e {
            SignerError::NotFound(_) => SignerError::KeyNotFound(key_id),
            other => other,
        })?;

    let response = SignResponse { signature };

    let duration = start_time.elapsed().as_secs_f64();
    KS_REQUEST_DURATION
        .with_label_values(&["signer", "POST", "/sign", "200"])
        .observe(duration);
    KS_REQUESTS_TOTAL
        .with_label_values(&["signer", "POST", "/sign", "200"])
        .inc();

    crate::recording::info!("Signed message for key_id: {}", key_id);
    Ok(Json(response))
}

async fn health_check_handler(
    State(app_state): State<SignerState>,
) -> Result<Json<serde_json::Value>, SignerError> {
    crate::recording::debug!("Health check requested");

    let key_count = app_state.storage.get_key_count().await?;

    let response = serde_json::json!({
        "status": "healthy",
        "service": "signer",
        "backend": app_state.storage.backend_name(),
        "key_count": key_count,
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    });

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use nonce_auth::{CredentialBuilder, NonceCredential, storage::MemoryStorage};
    use serde_json;
    use tempfile::tempdir;
    use tower::ServiceExt;

    async fn create_test_app() -> (Router, String, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let config = crate::config::SignerServiceConfig {
            storage: crate::storage::StorageConfig {
                backend: crate::storage::StorageBackend::Sqlite,
                key_ttl_seconds: 3600,
                sqlite: Some(crate::storage::SqliteConfig {}),
                postgres: None,
            },
            kek: None,
            kek_env: None,
            kek_file: None,
            tolerance_seconds: 3600,
        };

        let psk = "test-psk".to_string();
        let nonce_storage = MemoryStorage::new();
        let app_state = create_signer_state(&config, nonce_storage, &psk, temp_dir.path())
            .await
            .unwrap();

        let router = Router::new()
            .route("/generate-signing-key", post(generate_signing_key_handler))
            .route("/sign/{key_id}", post(sign_handler))
            .route("/health", get(health_check_handler))
            .with_state(app_state);
        (router, psk, temp_dir)
    }

    fn create_credential_for_request(psk: &str, request_data: &str) -> NonceCredential {
        CredentialBuilder::new(psk.as_bytes())
            .sign(request_data.as_bytes())
            .unwrap()
    }

    #[tokio::test]
    async fn test_service_creation_from_config() {
        let temp_dir = tempdir().unwrap();

        let config = crate::config::SignerServiceConfig {
            storage: crate::storage::StorageConfig {
                backend: crate::storage::StorageBackend::Sqlite,
                key_ttl_seconds: 3600,
                sqlite: Some(crate::storage::SqliteConfig {}),
                postgres: None,
            },
            kek: None,
            kek_env: None,
            kek_file: None,
            tolerance_seconds: 3600,
        };

        let nonce_storage = MemoryStorage::new();
        let state = create_signer_state(
            &config,
            nonce_storage,
            "test-shared-key-123",
            temp_dir.path(),
        )
        .await;
        assert!(state.is_ok());

        let state = state.unwrap();
        assert_eq!(state.psk, "test-shared-key-123");
    }

    #[tokio::test]
    async fn test_router_creation() {
        let temp_dir = tempdir().unwrap();

        let config = crate::config::SignerServiceConfig {
            storage: crate::storage::StorageConfig {
                backend: crate::storage::StorageBackend::Sqlite,
                key_ttl_seconds: 3600,
                sqlite: Some(crate::storage::SqliteConfig {}),
                postgres: None,
            },
            kek: None,
            kek_env: None,
            kek_file: None,
            tolerance_seconds: 3600,
        };

        let nonce_storage = MemoryStorage::new();
        let state = create_signer_state(&config, nonce_storage, "test-shared-key", temp_dir.path())
            .await
            .unwrap();

        let _router = create_router(state);
        // Router created successfully
    }

    #[tokio::test]
    async fn test_service_stats() {
        let temp_dir = tempdir().unwrap();

        let config = crate::config::SignerServiceConfig {
            storage: crate::storage::StorageConfig {
                backend: crate::storage::StorageBackend::Sqlite,
                key_ttl_seconds: 3600,
                sqlite: Some(crate::storage::SqliteConfig {}),
                postgres: None,
            },
            kek: None,
            kek_env: None,
            kek_file: None,
            tolerance_seconds: 3600,
        };

        let nonce_storage = MemoryStorage::new();
        let state = create_signer_state(&config, nonce_storage, "test-shared-key", temp_dir.path())
            .await
            .unwrap();

        let stats = get_stats(&state).await.unwrap();
        assert_eq!(stats.key_count, 0);
    }

    #[tokio::test]
    async fn test_health_check() {
        let (app, _psk, _temp_dir) = create_test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_generate_signing_key() {
        let (app, psk, _temp_dir) = create_test_app().await;

        let request_data = "generate_signing_key";
        let credential = create_credential_for_request(&psk, request_data);

        let request = GenerateSigningKeyRequest { credential };
        let request_body = serde_json::to_value(request).unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/generate-signing-key")
                    .header("content-type", "application/json")
                    .body(Body::from(request_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        if status != StatusCode::OK {
            let error_text = String::from_utf8_lossy(&body);
            eprintln!("Error response ({status}): {error_text}");
        }
        assert_eq!(status, StatusCode::OK);

        let response_json: GenerateSigningKeyResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(response_json.key_id, 1);
        assert!(!response_json.verifying_key.is_empty());
        assert_eq!(response_json.tolerance_seconds, 3600);
    }

    #[tokio::test]
    async fn test_invalid_signature() {
        let (app, psk, _temp_dir) = create_test_app().await;

        let invalid_credential = create_credential_for_request(&psk, "invalid-data");

        let invalid_request = GenerateSigningKeyRequest {
            credential: invalid_credential,
        };
        let request_body = serde_json::to_value(invalid_request).unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/generate-signing-key")
                    .header("content-type", "application/json")
                    .body(Body::from(request_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
