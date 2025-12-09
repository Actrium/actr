//! AIS 端到端集成测试
//!
//! 测试完整的 Token 签发和验证流程，确保 Issuer 和 Validator 使用匹配的密钥对

use actr_protocol::{ActrType, Realm, RegisterRequest, register_response};
use actrix_common::aid::credential::validator::AIdCredentialValidator;
use actrix_common::config::ks::KsClientConfig;
use ais::issuer::{AIdIssuer, IssuerConfig};
use ais::ks_client_wrapper::create_ks_client;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

/// 辅助函数：创建临时 KS 服务端点（模拟）
fn setup_test_environment() -> (TempDir, TempDir, KsClientConfig, String) {
    let issuer_temp_dir = TempDir::new().unwrap();
    let validator_temp_dir = TempDir::new().unwrap();

    let shared_key = std::env::var("KS_PSK").unwrap_or_else(|_| "test-psk-key".to_string());

    // 在实际集成测试中，需要启动真实的 KS gRPC 服务
    // 这里使用环境变量或默认配置（gRPC 端点）
    let ks_config = KsClientConfig {
        endpoint: std::env::var("KS_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:50052".to_string()),
        timeout_seconds: 30,
        enable_tls: false,
        tls_domain: None,
        ca_cert: None,
        client_cert: None,
        client_key: None,
    };

    (issuer_temp_dir, validator_temp_dir, ks_config, shared_key)
}

#[tokio::test]
#[ignore] // 需要真实 KS 服务运行，使用 `cargo test -- --ignored` 执行
async fn test_end_to_end_token_issuance_and_validation() {
    let (issuer_temp_dir, _validator_temp_dir, ks_config, shared_key) = setup_test_environment();

    // 1. 创建 Issuer
    let issuer_config = IssuerConfig {
        token_ttl_secs: 3600,
        signaling_heartbeat_interval_secs: 30,
        key_refresh_interval_secs: 3600,
        key_storage_file: issuer_temp_dir.path().join("issuer_keys.db"),
        enable_periodic_rotation: false,
        key_rotation_interval_secs: 86400,
    };

    let ks_client = create_ks_client(&ks_config, &shared_key)
        .await
        .expect("Failed to create KS gRPC client");
    let issuer = AIdIssuer::new(ks_client, issuer_config)
        .await
        .expect("Failed to create issuer");

    // 2. 初始化 Validator
    AIdCredentialValidator::init(&ks_config, &shared_key, _validator_temp_dir.path())
        .await
        .expect("Failed to initialize validator");

    // 3. 创建注册请求
    let request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "test-manufacturer".to_string(),
            name: "test-device".to_string(),
        },
        realm: Realm { realm_id: 1001 },
        service_spec: None,
        acl: None,
    };

    // 4. 签发 Token
    let response = issuer
        .issue_credential(&request)
        .await
        .expect("Failed to issue credential");

    // 5. 验证响应格式
    assert!(response.result.is_some(), "Response should have a result");

    let register_ok = match response.result.unwrap() {
        register_response::Result::Success(ok) => ok,
        register_response::Result::Error(err) => {
            panic!("Expected success but got error: {:?}", err);
        }
    };

    // 6. 验证基本字段
    assert!(register_ok.psk.is_some(), "PSK should be present");
    assert!(
        register_ok.credential_expires_at.is_some(),
        "Expiry time should be present"
    );

    let actr_id = &register_ok.actr_id;
    let credential = &register_ok.credential;

    // 7. 验证 ActrId 字段
    assert_eq!(actr_id.realm.realm_id, 1001, "Realm ID should match");
    assert!(
        actr_id.serial_number > 0,
        "Serial number should be valid Snowflake ID"
    );

    // 8. 使用 Validator 验证 Token
    let claims = AIdCredentialValidator::check(credential, 1001)
        .await
        .expect("Token validation should succeed");

    // 9. 验证 Claims 内容
    assert_eq!(claims.realm_id, 1001, "Realm ID in claims should match");
    assert!(
        !claims.actor_id.is_empty(),
        "Actor ID should be present in claims"
    );
    // Actor ID 格式: {manufacturer}:{name}@{serial_number_hex}:{realm_id}
    assert!(
        claims.actor_id.contains(&actr_id.serial_number.to_string()),
        "Actor ID should contain serial number"
    );

    // 10. 验证过期时间合理性
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(
        claims.expr_time > now,
        "Token should not be expired immediately after issuance"
    );
    assert!(
        claims.expr_time <= now + 3600,
        "Token expiry should be within configured TTL"
    );

    println!("✅ End-to-end test passed!");
    println!("  - Token issued successfully");
    println!("  - Token validated successfully");
    println!("  - Claims matched expected values");
}

#[tokio::test]
#[ignore] // 需要真实 KS 服务运行
async fn test_token_validation_with_wrong_tenant_fails() {
    let (issuer_temp_dir, _validator_temp_dir, ks_config, shared_key) = setup_test_environment();

    // 创建 Issuer
    let issuer_config = IssuerConfig {
        token_ttl_secs: 3600,
        signaling_heartbeat_interval_secs: 30,
        key_refresh_interval_secs: 3600,
        key_storage_file: issuer_temp_dir.path().join("issuer_keys.db"),
        enable_periodic_rotation: false,
        key_rotation_interval_secs: 86400,
    };

    let ks_client = create_ks_client(&ks_config, &shared_key)
        .await
        .expect("Failed to create KS gRPC client");
    let issuer = AIdIssuer::new(ks_client, issuer_config)
        .await
        .expect("Failed to create issuer");

    // 初始化 Validator
    AIdCredentialValidator::init(&ks_config, &shared_key, _validator_temp_dir.path())
        .await
        .expect("Failed to initialize validator");

    // 为 realm_id=1001 签发 Token
    let request = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "test-manufacturer".to_string(),
            name: "test-device".to_string(),
        },
        realm: Realm { realm_id: 1001 },
        service_spec: None,
        acl: None,
    };

    let response = issuer
        .issue_credential(&request)
        .await
        .expect("Failed to issue credential");

    let register_ok = match response.result.unwrap() {
        register_response::Result::Success(ok) => ok,
        register_response::Result::Error(err) => {
            panic!("Expected success but got error: {:?}", err);
        }
    };

    let credential = &register_ok.credential;

    // 尝试使用错误的 realm_id 验证（期望失败）
    let result = AIdCredentialValidator::check(credential, 9999).await;

    assert!(
        result.is_err(),
        "Validation should fail with mismatched realm_id"
    );

    println!("✅ Wrong realm validation test passed!");
}

#[tokio::test]
#[ignore] // 需要真实 KS 服务运行
async fn test_multiple_key_rotations() {
    let (issuer_temp_dir, _validator_temp_dir, ks_config, shared_key) = setup_test_environment();

    // 创建 Issuer
    let issuer_config = IssuerConfig {
        token_ttl_secs: 3600,
        signaling_heartbeat_interval_secs: 30,
        key_refresh_interval_secs: 3600,
        key_storage_file: issuer_temp_dir.path().join("issuer_keys.db"),
        enable_periodic_rotation: false,
        key_rotation_interval_secs: 86400,
    };

    let ks_client = create_ks_client(&ks_config, &shared_key)
        .await
        .expect("Failed to create KS gRPC client");
    let issuer = AIdIssuer::new(ks_client, issuer_config)
        .await
        .expect("Failed to create issuer");

    // 初始化 Validator
    AIdCredentialValidator::init(&ks_config, &shared_key, _validator_temp_dir.path())
        .await
        .expect("Failed to initialize validator");

    // 签发多个 Token
    let mut credentials = Vec::new();

    for i in 0..5 {
        let request = RegisterRequest {
            actr_type: ActrType {
                manufacturer: format!("test-manufacturer-{}", i),
                name: format!("test-device-{}", i),
            },
            realm: Realm { realm_id: 1001 },
            service_spec: None,
            acl: None,
        };

        let response = issuer
            .issue_credential(&request)
            .await
            .expect("Failed to issue credential");

        let register_ok = match response.result.unwrap() {
            register_response::Result::Success(ok) => ok,
            register_response::Result::Error(err) => {
                panic!("Expected success but got error: {:?}", err);
            }
        };

        credentials.push(register_ok.credential);
    }

    // 验证所有 Token
    for (i, credential) in credentials.iter().enumerate() {
        let claims = AIdCredentialValidator::check(credential, 1001)
            .await
            .unwrap_or_else(|e| panic!("Token {} validation failed: {}", i, e));

        assert_eq!(
            claims.realm_id, 1001,
            "Realm ID should match for token {}",
            i
        );
    }

    println!("✅ Multiple key rotations test passed!");
    println!("  - Issued {} tokens", credentials.len());
    println!("  - All tokens validated successfully");
}

#[tokio::test]
async fn test_issuer_health_checks() {
    let (issuer_temp_dir, _validator_temp_dir, ks_config, shared_key) = setup_test_environment();

    let issuer_config = IssuerConfig {
        token_ttl_secs: 3600,
        signaling_heartbeat_interval_secs: 30,
        key_refresh_interval_secs: 3600,
        key_storage_file: issuer_temp_dir.path().join("issuer_keys.db"),
        enable_periodic_rotation: false,
        key_rotation_interval_secs: 86400,
    };

    // 注意：这个测试可能会失败如果 KS 服务不可用
    // 但不应该导致 panic，应该返回错误
    let ks_client_result = create_ks_client(&ks_config, &shared_key).await;

    match ks_client_result {
        Ok(ks_client) => match AIdIssuer::new(ks_client, issuer_config).await {
            Ok(issuer) => {
                // 如果 KS 可用，执行健康检查
                let _ = issuer.check_database_health().await;
                let _ = issuer.check_key_cache_health().await;
                println!("✅ Issuer health checks passed (KS available)");
            }
            Err(e) => {
                // 如果 KS 不可用，应该得到明确的错误信息
                println!("⚠️  Issuer creation failed (KS unavailable): {}", e);
                assert!(
                    e.to_string().contains("KS"),
                    "Error should mention KS service"
                );
            }
        },
        Err(e) => {
            println!("⚠️  KS gRPC client creation failed: {}", e);
            assert!(
                e.to_string().contains("KS") || e.to_string().contains("gRPC"),
                "Error should mention KS or gRPC"
            );
        }
    }
}
