//! 集成测试：PSK Bootstrap 两阶段全流程
//!
//! 覆盖场景：
//! 1. 首次注册：无 PSK → manifest auth → AIS 下发 credential + PSK → 存储
//! 2. PSK 续期：有效 PSK → PSK auth → AIS 下发新 credential（无新 PSK）
//! 3. PSK 过期：PSK 过期 → 回落到 manifest auth → AIS 下发 credential + 新 PSK
//! 4. PSK 更新：首次注册后再次注册 → 使用新拿到的 PSK
//! 5. AIS 错误：AIS 返回错误 → 正确传播 HyperError

use std::time::{SystemTime, UNIX_EPOCH};

use actr_hyper::{ActorStore, Hyper, HyperConfig, HyperError, PackageManifest, TrustMode};
use ed25519_dalek::SigningKey;
use prost::Message;
use rand::rngs::OsRng;
use tempfile::TempDir;

// ─── 辅助函数 ─────────────────────────────────────────────────────────────────

fn dev_config(dir: &TempDir) -> HyperConfig {
    let signing_key = SigningKey::generate(&mut OsRng);
    let pubkey = signing_key.verifying_key().to_bytes().to_vec();
    HyperConfig::new(dir.path()).with_trust_mode(TrustMode::Development {
        self_signed_pubkey: pubkey,
    })
}

fn fake_manifest() -> PackageManifest {
    PackageManifest {
        manufacturer: "test-mfr".to_string(),
        actr_name: "TestActor".to_string(),
        version: "0.1.0".to_string(),
        binary_hash: [0u8; 32],
        capabilities: vec![],
        signature: vec![0u8; 64],
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// 构建合法的 RegisterResponse protobuf bytes
fn make_register_response(with_psk: bool, psk_bytes: Option<&[u8]>) -> Vec<u8> {
    use actr_protocol::{
        AIdCredential, ActrId, ActrType, IdentityClaims, Realm, RegisterResponse, TurnCredential,
        register_response,
    };

    let claims = IdentityClaims {
        realm_id: 1,
        actor_id: "test-actor-id".to_string(),
        expires_at: u64::MAX,
    };
    let credential = AIdCredential {
        key_id: 1,
        claims: claims.encode_to_vec().into(),
        signature: vec![0u8; 64].into(),
    };
    let actr_id = ActrId {
        realm: Realm { realm_id: 1 },
        serial_number: 42,
        r#type: ActrType {
            manufacturer: "test-mfr".to_string(),
            name: "TestActor".to_string(),
            version: "0.1.0".to_string(),
        },
    };
    let turn = TurnCredential {
        username: "u".to_string(),
        password: "p".to_string(),
        expires_at: u64::MAX,
    };

    let mut ok = register_response::RegisterOk {
        actr_id,
        credential,
        turn_credential: turn,
        credential_expires_at: None,
        signaling_heartbeat_interval_secs: 30,
        signing_pubkey: vec![0u8; 32].into(),
        signing_key_id: 1,
        psk: None,
        psk_expires_at: None,
    };

    if with_psk {
        let psk = psk_bytes.unwrap_or(b"server-generated-psk");
        ok.psk = Some(psk.to_vec().into());
        ok.psk_expires_at = Some((now_secs() + 86400) as i64); // 24 小时后过期
    }

    RegisterResponse {
        result: Some(register_response::Result::Success(ok)),
    }
    .encode_to_vec()
}

fn make_error_response(code: u32, message: &str) -> Vec<u8> {
    use actr_protocol::{ErrorResponse, RegisterResponse, register_response};
    RegisterResponse {
        result: Some(register_response::Result::Error(ErrorResponse {
            code,
            message: message.to_string(),
        })),
    }
    .encode_to_vec()
}

// ─── 测试用例 ─────────────────────────────────────────────────────────────────

/// 场景 1：首次注册（ActorStore 无 PSK）→ manifest auth → 收到 credential + PSK → 存储
#[tokio::test]
async fn first_registration_uses_manifest_auth_and_stores_psk() {
    let psk = b"initial-psk-token";
    let resp_body = make_register_response(true, Some(psk));

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(resp_body)
        .expect(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();
    let manifest = fake_manifest();

    let credential = hyper
        .bootstrap_credential(&manifest, &server.url(), 1)
        .await
        .unwrap();

    mock.assert_async().await;
    assert!(!credential.is_empty(), "credential 不应为空");

    // PSK 应已写入 ActorStore
    let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
    let store = ActorStore::open(&storage_path).await.unwrap();
    let stored_psk = store.kv_get("hyper:psk:token").await.unwrap();
    assert_eq!(
        stored_psk,
        Some(psk.to_vec()),
        "PSK 应已持久化"
    );

    // expires_at 也应有效
    let expires = store.kv_get("hyper:psk:expires_at").await.unwrap();
    assert!(expires.is_some(), "PSK expires_at 应已持久化");
    let expires_secs = u64::from_le_bytes(expires.unwrap().try_into().unwrap());
    assert!(expires_secs > now_secs(), "PSK 应未过期");
}

/// 场景 2：有有效 PSK → PSK auth → 仅发一次 /register → 无新 PSK 下发
#[tokio::test]
async fn valid_psk_uses_psk_auth_without_new_psk() {
    let resp_body = make_register_response(false, None);

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(resp_body)
        .expect(1) // 恰好调用一次
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();
    let manifest = fake_manifest();

    // 预先写入有效 PSK
    let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
    let store = ActorStore::open(&storage_path).await.unwrap();
    let valid_psk = b"existing-valid-psk";
    store.kv_set("hyper:psk:token", valid_psk).await.unwrap();
    store
        .kv_set("hyper:psk:expires_at", &(now_secs() + 3600).to_le_bytes())
        .await
        .unwrap();

    let credential = hyper
        .bootstrap_credential(&manifest, &server.url(), 1)
        .await
        .unwrap();

    mock.assert_async().await;
    assert!(!credential.is_empty());

    // PSK 应保持不变（无新 PSK 下发）
    let stored = store.kv_get("hyper:psk:token").await.unwrap();
    assert_eq!(stored, Some(valid_psk.to_vec()), "PSK 应保持原值");
}

/// 场景 3：PSK 过期 → 回落到 manifest auth → 收到新 PSK
#[tokio::test]
async fn expired_psk_falls_back_to_manifest_and_receives_new_psk() {
    let new_psk = b"renewed-psk-after-expiry";
    let resp_body = make_register_response(true, Some(new_psk));

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(resp_body)
        .expect(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();
    let manifest = fake_manifest();

    // 预先写入过期 PSK（10 秒前）
    let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
    let store = ActorStore::open(&storage_path).await.unwrap();
    store
        .kv_set("hyper:psk:token", b"old-expired-psk")
        .await
        .unwrap();
    store
        .kv_set(
            "hyper:psk:expires_at",
            &now_secs().saturating_sub(10).to_le_bytes(),
        )
        .await
        .unwrap();

    hyper
        .bootstrap_credential(&manifest, &server.url(), 1)
        .await
        .unwrap();

    mock.assert_async().await;

    // 新 PSK 应覆盖旧的
    let stored = store.kv_get("hyper:psk:token").await.unwrap();
    assert_eq!(
        stored,
        Some(new_psk.to_vec()),
        "过期后应收到并存储新 PSK"
    );
}

/// 场景 4：连续两次注册（首次 + 续期）→ 第一次用 manifest，第二次用 PSK
#[tokio::test]
async fn sequential_registrations_switch_from_manifest_to_psk() {
    let psk = b"sequential-psk";
    let first_resp = make_register_response(true, Some(psk));
    let second_resp = make_register_response(false, None);

    let mut server = mockito::Server::new_async().await;

    // 第一次：返回 PSK
    let mock1 = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(first_resp)
        .expect(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();
    let manifest = fake_manifest();

    // 第一次注册（manifest auth）
    hyper
        .bootstrap_credential(&manifest, &server.url(), 1)
        .await
        .unwrap();
    mock1.assert_async().await;

    // 第二次：返回 credential，无新 PSK
    let mock2 = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(second_resp)
        .expect(1)
        .create_async()
        .await;

    // 第二次注册（PSK auth，因为第一次已存储 PSK）
    hyper
        .bootstrap_credential(&manifest, &server.url(), 1)
        .await
        .unwrap();
    mock2.assert_async().await;
}

/// 场景 5：AIS 返回 403 → 传播 HyperError::AisBootstrapFailed
#[tokio::test]
async fn ais_error_propagates_as_bootstrap_failed() {
    let error_resp = make_error_response(403u32, "manufacturer not registered");

    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(error_resp)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();

    let result = hyper
        .bootstrap_credential(&fake_manifest(), &server.url(), 1)
        .await;

    assert!(
        matches!(result, Err(HyperError::AisBootstrapFailed(_))),
        "AIS 错误应传播为 AisBootstrapFailed，实际: {result:?}"
    );
}

/// 场景 6：AIS 不可达（连接被拒）→ 传播 HyperError::AisBootstrapFailed（网络错误）
#[tokio::test]
async fn ais_unreachable_propagates_error() {
    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();

    // 使用无效端口
    let result = hyper
        .bootstrap_credential(&fake_manifest(), "http://127.0.0.1:19999", 1)
        .await;

    assert!(
        result.is_err(),
        "AIS 不可达时应返回错误，实际: {result:?}"
    );
}

/// PSK 和 signing_pubkey 都应在首次注册后持久化
#[tokio::test]
async fn first_registration_persists_signing_pubkey() {
    let resp_body = make_register_response(true, Some(b"my-psk"));

    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/register")
        .with_status(200)
        .with_header("content-type", "application/x-protobuf")
        .with_body(resp_body)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let hyper = Hyper::init(dev_config(&dir)).await.unwrap();
    let manifest = fake_manifest();

    hyper
        .bootstrap_credential(&manifest, &server.url(), 1)
        .await
        .unwrap();

    let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
    let store = ActorStore::open(&storage_path).await.unwrap();

    let pubkey = store.kv_get("hyper:ais:signing_pubkey").await.unwrap();
    assert!(pubkey.is_some(), "signing_pubkey 应已持久化");
    assert_eq!(pubkey.unwrap().len(), 32, "Ed25519 pubkey 应为 32 字节");

    let key_id = store.kv_get("hyper:ais:signing_key_id").await.unwrap();
    assert!(key_id.is_some(), "signing_key_id 应已持久化");
}
