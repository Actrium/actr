//! # actr-hyper
//!
//! Hyper — Actor 运行平台层
//!
//! ## 定位
//!
//! Hyper 是 Actor 的操作系统：划定边界（Sandbox），提供平台原语。
//! Actor 不能自己打开数据库，不能自己持有私钥，不能声称自己是某个类型——
//! 一切都必须经过 Hyper 的受控接口。
//!
//! ## 职责
//!
//! - 包签名验证（binary_hash + MFR 签名）
//! - Actor 启动引导（代表 Actor 向 AIS 发起注册，取得 credential）
//! - 存储命名空间隔离（每个 Actor 独立的 SQLite 空间）
//! - 加密原语（Ed25519 签名/验证，Actor 不持有原始私钥）
//! - 运行时管理（三种模式的 ActrSystem 生命周期）
//!
//! ## 不做的事
//!
//! Hyper 不理解业务，不做消息路由，不知道 Actor 之间的关系。
//! WASM 模式下提供的 `hyper_send`/`hyper_recv` 是网络 I/O 原语，
//! 路由决策由 WASM 内的 ActrSystem 完成。

pub mod ais_client;
pub mod config;
pub mod error;
pub mod key_cache;
pub mod runtime;
pub mod storage;
pub mod verify;


pub use ais_client::AisClient;
pub use config::{HyperConfig, TrustMode};
pub use error::{HyperError, HyperResult};
pub use runtime::{ActorRuntime, ActrSystemHandle, ChildProcessHandle, WasmInstanceHandle};
pub use storage::ActorStore;
pub use verify::{
    MfrCertCache, PackageManifest,
    embed_elf_manifest, embed_macho_manifest, embed_wasm_manifest,
    manifest_signed_bytes,
};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use prost::Message;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use actr_protocol::{register_response, ActrType, Realm, RegisterRequest};
use storage::KvOp;

/// Hyper 运行时实例
///
/// 进程级单例，通过 `Hyper::init()` 初始化。
/// 持有已解析的配置、instance_id、命名空间解析器等进程级状态。
#[derive(Clone)]
pub struct Hyper {
    inner: Arc<HyperInner>,
}

struct HyperInner {
    config: HyperConfig,
    /// 首次启动时生成并持久化的本地唯一 ID
    instance_id: String,
    /// 包签名验证器
    verifier: verify::PackageVerifier,
}

impl Hyper {
    /// 初始化 Hyper（进程级，调用一次）
    ///
    /// - 解析配置
    /// - 加载或生成 instance_id（持久化到 data_dir）
    /// - 初始化包验证器
    pub async fn init(config: HyperConfig) -> HyperResult<Self> {
        info!(
            data_dir = %config.data_dir.display(),
            trust_mode = match &config.trust_mode {
                TrustMode::Production { .. } => "production",
                TrustMode::Development { .. } => "development",
            },
            "Hyper 初始化"
        );

        // 确保 data_dir 存在
        tokio::fs::create_dir_all(&config.data_dir).await.map_err(|e| {
            HyperError::Config(format!(
                "无法创建 data_dir `{}`: {e}",
                config.data_dir.display()
            ))
        })?;

        // 加载或生成 instance_id
        let instance_id = load_or_create_instance_id(&config.data_dir).await?;
        debug!(instance_id, "Hyper instance_id 已就绪");

        let verifier = verify::PackageVerifier::new(config.trust_mode.clone());

        Ok(Self {
            inner: Arc::new(HyperInner {
                config,
                instance_id,
                verifier,
            }),
        })
    }

    /// 验证 ActrPackage 字节流，返回已验证的 manifest
    ///
    /// 验证通过意味着：
    /// - binary_hash 与重算结果一致（包未被篡改）
    /// - MFR 签名合法（来自可信制造商）
    ///
    /// 生产模式下会先异步拉取 MFR 公钥（需要 AIS 可达）；
    /// 开发模式下无网络调用，完全同步。
    pub async fn verify_package(&self, bytes: &[u8]) -> HyperResult<PackageManifest> {
        // 生产模式：先预取 MFR 公钥（写入 cert_cache），再同步验证
        if matches!(&self.inner.config.trust_mode, TrustMode::Production { .. }) {
            if let Some(manufacturer) = quick_extract_manufacturer(bytes) {
                debug!(manufacturer, "生产模式：预取 MFR 公钥");
                self.inner.verifier.prefetch_mfr_cert(&manufacturer).await?;
            }
        }
        self.inner.verifier.verify(bytes)
    }

    /// 为已验证的 manifest 解析存储命名空间路径
    ///
    /// 路径在此处固定，后续所有存储操作都基于此路径做隔离。
    pub fn resolve_storage_path(&self, manifest: &PackageManifest) -> HyperResult<PathBuf> {
        let resolver = config::NamespaceResolver::new(&self.inner.config, &self.inner.instance_id)?
            .with_actor_type(&manifest.manufacturer, &manifest.actr_name, &manifest.version);
        resolver.resolve(&self.inner.config.storage_path_template)
    }

    /// 向 AIS 发起 credential 注册引导（两阶段流程）
    ///
    /// Hyper 代表 Actor 完成注册引导，取得 ActrId credential。
    /// 此 credential 随后传递给 ActrSystem（Mode 1/3）或通过环境变量传递给子进程（Mode 2）。
    ///
    /// ## 两阶段逻辑
    ///
    /// - **阶段 1（首次注册）**：ActorStore 中无有效 PSK →
    ///   用 MFR 签名 manifest 注册 → AIS 返回 credential + PSK → 存入 ActorStore
    /// - **阶段 2（PSK 续期）**：ActorStore 中有有效 PSK →
    ///   直接用 PSK 注册 → AIS 返回新 credential
    ///
    /// ## 参数
    ///
    /// - `manifest`: 已验证的包 manifest（来自 `verify_package`）
    /// - `ais_endpoint`: AIS HTTP 地址，例如 `"http://ais.example.com:8080"`
    /// - `realm_id`: 目标 Realm ID
    pub async fn bootstrap_credential(
        &self,
        manifest: &PackageManifest,
        ais_endpoint: &str,
        realm_id: u32,
    ) -> HyperResult<Vec<u8>> {
        info!(
            actr_type = manifest.actr_type_str(),
            ais_endpoint,
            realm_id,
            "开始向 AIS 申请 credential"
        );

        // 1. 打开该 Actor 的 ActorStore（存储命名空间隔离）
        let storage_path = self.resolve_storage_path(manifest)?;
        let store = ActorStore::open(&storage_path).await?;

        // 2. 检查 ActorStore 中是否有有效 PSK
        let valid_psk = load_valid_psk(&store).await?;

        // 3. 构造 RegisterRequest 并发送到 AIS
        let ais = AisClient::new(ais_endpoint);

        let actr_type = ActrType {
            manufacturer: manifest.manufacturer.clone(),
            name: manifest.actr_name.clone(),
            version: manifest.version.clone(),
        };
        let realm = Realm { realm_id };

        let response = if let Some(psk_token) = valid_psk {
            // 阶段 2：PSK 续期
            debug!(
                actr_type = manifest.actr_type_str(),
                "使用 PSK 续期 credential"
            );
            let req = RegisterRequest {
                actr_type,
                realm,
                service_spec: None,
                acl: None,
                service: None,
                ws_address: None,
                manifest_json: None,
                mfr_signature: None,
                psk_token: Some(psk_token.into()),
            };
            ais.register_with_psk(req).await?
        } else {
            // 阶段 1：首次注册，携带 MFR manifest
            info!(
                actr_type = manifest.actr_type_str(),
                "首次注册：使用 MFR manifest 向 AIS 注册"
            );

            // 序列化 manifest 为 JSON
            let manifest_json = build_manifest_json(manifest)?;

            let req = RegisterRequest {
                actr_type,
                realm,
                service_spec: None,
                acl: None,
                service: None,
                ws_address: None,
                manifest_json: Some(manifest_json.into()),
                mfr_signature: Some(manifest.signature.clone().into()),
                psk_token: None,
            };
            ais.register_with_manifest(req).await?
        };

        // 4. 处理 AIS 响应
        let ok = match response.result {
            Some(register_response::Result::Success(ok)) => ok,
            Some(register_response::Result::Error(e)) => {
                error!(
                    actr_type = manifest.actr_type_str(),
                    error_code = e.code,
                    error_message = %e.message,
                    "AIS 注册返回错误"
                );
                return Err(HyperError::AisBootstrapFailed(format!(
                    "AIS 拒绝注册 (code={}): {}",
                    e.code, e.message
                )));
            }
            None => {
                error!(
                    actr_type = manifest.actr_type_str(),
                    "AIS 响应中缺少 result 字段"
                );
                return Err(HyperError::AisBootstrapFailed(
                    "AIS 响应缺少 result 字段".to_string(),
                ));
            }
        };

        // 5a. 若响应中含有 PSK（首次注册场景），存入 ActorStore
        if let (Some(psk), Some(psk_expires_at)) = (&ok.psk, ok.psk_expires_at) {
            info!(
                actr_type = manifest.actr_type_str(),
                psk_expires_at,
                "收到 AIS 下发的 PSK，存入 ActorStore"
            );
            let expires_at_bytes = (psk_expires_at as u64).to_le_bytes().to_vec();
            store
                .kv_batch(vec![
                    KvOp::Set {
                        key: "hyper:psk:token".to_string(),
                        value: psk.to_vec(),
                    },
                    KvOp::Set {
                        key: "hyper:psk:expires_at".to_string(),
                        value: expires_at_bytes,
                    },
                ])
                .await?;
            debug!(
                actr_type = manifest.actr_type_str(),
                "PSK 已成功持久化到 ActorStore"
            );
        }

        // 5b. 存储 signing_pubkey + signing_key_id（供 AisKeyCache 使用）
        let pubkey_bytes = ok.signing_pubkey.to_vec();
        let key_id_bytes = ok.signing_key_id.to_le_bytes().to_vec();
        store
            .kv_batch(vec![
                KvOp::Set {
                    key: "hyper:ais:signing_pubkey".to_string(),
                    value: pubkey_bytes,
                },
                KvOp::Set {
                    key: "hyper:ais:signing_key_id".to_string(),
                    value: key_id_bytes,
                },
            ])
            .await?;
        debug!(
            actr_type = manifest.actr_type_str(),
            signing_key_id = ok.signing_key_id,
            "AIS 签名公钥已持久化到 ActorStore"
        );

        // 6. 序列化 AIdCredential 并返回（credential 是 required 字段，直接使用）
        let credential_bytes = ok.credential.encode_to_vec();
        info!(
            actr_type = manifest.actr_type_str(),
            credential_len = credential_bytes.len(),
            "AIS credential 引导成功"
        );

        Ok(credential_bytes)
    }

    /// 当前 instance_id
    pub fn instance_id(&self) -> &str {
        &self.inner.instance_id
    }

    /// 当前配置
    pub fn config(&self) -> &HyperConfig {
        &self.inner.config
    }
}

// ─── 辅助函数 ────────────────────────────────────────────────────────────────

/// 从 ActorStore 中读取 PSK，若存在且未过期则返回 PSK bytes，否则返回 None
///
/// PSK 过期检查：当前 Unix 时间戳（秒）≥ expires_at 时视为已过期。
async fn load_valid_psk(store: &ActorStore) -> HyperResult<Option<Vec<u8>>> {
    let token = store.kv_get("hyper:psk:token").await?;
    let expires_at_raw = store.kv_get("hyper:psk:expires_at").await?;

    match (token, expires_at_raw) {
        (Some(token), Some(expires_bytes)) => {
            // 解析过期时间（u64 little-endian）
            if expires_bytes.len() != 8 {
                warn!("PSK expires_at 格式异常，将重走首次注册流程");
                return Ok(None);
            }
            let expires_at = u64::from_le_bytes(expires_bytes.as_slice().try_into().unwrap());

            // 获取当前 Unix 时间戳（秒）
            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            if now_secs >= expires_at {
                warn!(
                    psk_expires_at = expires_at,
                    now = now_secs,
                    "PSK 已过期，将重走首次注册流程"
                );
                Ok(None)
            } else {
                debug!(
                    psk_expires_at = expires_at,
                    now = now_secs,
                    remaining_secs = expires_at - now_secs,
                    "PSK 有效，使用 PSK 续期路径"
                );
                Ok(Some(token))
            }
        }
        _ => {
            debug!("ActorStore 中无 PSK，走首次注册流程");
            Ok(None)
        }
    }
}

/// 将 PackageManifest 序列化为 JSON bytes
///
/// AIS 侧通过 manifest_json + mfr_signature 验证 MFR 身份。
/// 包含 signature（base64 编码）和 binary_hash（hex 编码），与原始包格式一致。
fn build_manifest_json(manifest: &PackageManifest) -> HyperResult<Vec<u8>> {
    use base64::Engine;

    let binary_hash_hex: String = manifest
        .binary_hash
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let sig_b64 =
        base64::engine::general_purpose::STANDARD.encode(&manifest.signature);

    let json = serde_json::json!({
        "manufacturer": manifest.manufacturer,
        "actr_name": manifest.actr_name,
        "version": manifest.version,
        "binary_hash": binary_hash_hex,
        "capabilities": manifest.capabilities,
        "signature": sig_b64,
    });

    serde_json::to_vec(&json).map_err(|e| {
        HyperError::AisBootstrapFailed(format!("manifest 序列化为 JSON 失败: {e}"))
    })
}

/// 从二进制文件中快速提取 manufacturer 字段（仅用于预取 MFR 公钥，不做完整验证）
///
/// 从 manifest section（WASM/ELF/Mach-O）中解析 JSON，提取 manufacturer 字段。
/// 解析失败或格式不识别时返回 None（调用方会跳过预取，verify 时再报错）。
fn quick_extract_manufacturer(bytes: &[u8]) -> Option<String> {
    use verify::manifest::{
        extract_wasm_manifest, extract_elf_manifest, extract_macho_manifest,
        is_wasm, is_elf, is_macho,
    };

    let section = if is_wasm(bytes) {
        extract_wasm_manifest(bytes)?
    } else if is_elf(bytes) {
        extract_elf_manifest(bytes)?
    } else if is_macho(bytes) {
        extract_macho_manifest(bytes)?
    } else {
        return None;
    };

    let value: serde_json::Value = serde_json::from_slice(section).ok()?;
    value["manufacturer"].as_str().map(|s| s.to_string())
}

/// 加载已有 instance_id 或生成新的并持久化
async fn load_or_create_instance_id(data_dir: &std::path::Path) -> HyperResult<String> {
    let id_file = data_dir.join(".hyper-instance-id");

    if id_file.exists() {
        let id = tokio::fs::read_to_string(&id_file).await.map_err(|e| {
            HyperError::Storage(format!("读取 instance_id 文件失败: {e}"))
        })?;
        let id = id.trim().to_string();
        if !id.is_empty() {
            return Ok(id);
        }
        warn!("instance_id 文件为空，重新生成");
    }

    let new_id = Uuid::new_v4().to_string();
    tokio::fs::write(&id_file, &new_id).await.map_err(|e| {
        HyperError::Storage(format!("写入 instance_id 文件失败: {e}"))
    })?;
    info!(instance_id = %new_id, "生成新的 Hyper instance_id");
    Ok(new_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use tempfile::TempDir;

    fn dev_config(dir: &TempDir) -> HyperConfig {
        let signing_key = SigningKey::generate(&mut OsRng);
        let pubkey = signing_key.verifying_key().to_bytes().to_vec();
        HyperConfig::new(dir.path()).with_trust_mode(TrustMode::Development {
            self_signed_pubkey: pubkey,
        })
    }

    #[tokio::test]
    async fn init_creates_data_dir_and_instance_id() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("subdir/nested");
        let config = dev_config(&TempDir::new().unwrap());
        let config = HyperConfig::new(&sub).with_trust_mode(config.trust_mode);

        let hyper = Hyper::init(config).await.unwrap();
        assert!(sub.exists());
        assert!(!hyper.instance_id().is_empty());
    }

    #[tokio::test]
    async fn instance_id_is_stable_across_reinit() {
        let dir = TempDir::new().unwrap();
        let config1 = dev_config(&dir);
        let hyper1 = Hyper::init(config1).await.unwrap();
        let id1 = hyper1.instance_id().to_string();

        let config2 = dev_config(&dir);
        let hyper2 = Hyper::init(config2).await.unwrap();
        let id2 = hyper2.instance_id().to_string();

        assert_eq!(id1, id2, "重启后 instance_id 应保持不变");
    }

    #[tokio::test]
    async fn verify_package_rejects_non_wasm() {
        let dir = TempDir::new().unwrap();
        let hyper = Hyper::init(dev_config(&dir)).await.unwrap();
        let result = hyper.verify_package(b"not a wasm file").await;
        assert!(matches!(result, Err(HyperError::InvalidManifest(_))));
    }

    #[tokio::test]
    async fn verify_package_rejects_wasm_without_manifest() {
        let dir = TempDir::new().unwrap();
        let hyper = Hyper::init(dev_config(&dir)).await.unwrap();

        // 最小合法 WASM（只有 magic + version，无 section）
        let minimal_wasm = b"\0asm\x01\x00\x00\x00";
        let result = hyper.verify_package(minimal_wasm).await;
        assert!(matches!(result, Err(HyperError::ManifestNotFound)));
    }

    // ─── PSK 存取与过期检查单元测试 ──────────────────────────────────────────

    async fn open_test_store(dir: &TempDir) -> ActorStore {
        let db_path = dir.path().join("test.db");
        ActorStore::open(&db_path).await.unwrap()
    }

    /// 写入有效 PSK，验证 load_valid_psk 返回该 PSK
    #[tokio::test]
    async fn psk_valid_returns_token() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        let psk_token = b"test-psk-secret".to_vec();
        // 过期时间设为当前时间 + 3600 秒（1小时后）
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;

        store.kv_set("hyper:psk:token", &psk_token).await.unwrap();
        store
            .kv_set("hyper:psk:expires_at", &expires_at.to_le_bytes())
            .await
            .unwrap();

        let result = load_valid_psk(&store).await.unwrap();
        assert_eq!(result, Some(psk_token), "有效 PSK 应被返回");
    }

    /// 写入已过期 PSK，验证 load_valid_psk 返回 None
    #[tokio::test]
    async fn psk_expired_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        let psk_token = b"expired-psk".to_vec();
        // 过期时间设为过去（1 秒前）
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(1);

        store.kv_set("hyper:psk:token", &psk_token).await.unwrap();
        store
            .kv_set("hyper:psk:expires_at", &expires_at.to_le_bytes())
            .await
            .unwrap();

        let result = load_valid_psk(&store).await.unwrap();
        assert_eq!(result, None, "已过期 PSK 应返回 None");
    }

    /// ActorStore 中无 PSK 时，load_valid_psk 返回 None
    #[tokio::test]
    async fn psk_absent_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        let result = load_valid_psk(&store).await.unwrap();
        assert_eq!(result, None, "无 PSK 时应返回 None");
    }

    /// 仅有 token 没有 expires_at 时，load_valid_psk 返回 None
    #[tokio::test]
    async fn psk_missing_expires_at_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = open_test_store(&dir).await;

        store
            .kv_set("hyper:psk:token", b"orphan-token")
            .await
            .unwrap();
        // 故意不写 expires_at

        let result = load_valid_psk(&store).await.unwrap();
        assert_eq!(result, None, "缺少 expires_at 时应返回 None");
    }

    /// build_manifest_json 产出的 JSON 含必要字段，可被 serde_json 解析
    #[test]
    fn build_manifest_json_produces_valid_json() {
        let manifest = PackageManifest {
            manufacturer: "acme".to_string(),
            actr_name: "Sensor".to_string(),
            version: "1.0.0".to_string(),
            binary_hash: [0u8; 32],
            capabilities: vec!["storage".to_string()],
            signature: vec![0u8; 64],
        };

        let json_bytes = build_manifest_json(&manifest).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();

        assert_eq!(value["manufacturer"], "acme");
        assert_eq!(value["actr_name"], "Sensor");
        assert_eq!(value["version"], "1.0.0");
        // binary_hash 应为 64 位 hex 字符串
        assert_eq!(
            value["binary_hash"].as_str().unwrap().len(),
            64,
            "binary_hash 应为 64 字符 hex"
        );
        assert!(value["capabilities"].is_array());
        assert!(value["signature"].is_string(), "signature 应为 base64 字符串");
    }

    // ─── AIS 集成测试（mockito mock server）──────────────────────────────────

    /// 辅助函数：构造测试用 PackageManifest
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

    /// 辅助函数：构造合法的 RegisterResponse protobuf bytes（含 credential）
    fn fake_register_response_bytes(with_psk: bool) -> Vec<u8> {
        use actr_protocol::{
            AIdCredential, ActrId, ActrType, IdentityClaims, Realm, RegisterResponse,
            TurnCredential, register_response,
        };

        let claims = IdentityClaims {
            realm_id: 1,
            actor_id: "test-actor-id".to_string(),
            expires_at: u64::MAX,
        };
        let claims_bytes = claims.encode_to_vec();

        let credential = AIdCredential {
            key_id: 1,
            claims: claims_bytes.into(),
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
            username: "user".to_string(),
            password: "pass".to_string(),
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
            ok.psk = Some(b"fresh-psk-from-ais".to_vec().into());
            ok.psk_expires_at = Some(
                (SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 86400) as i64,
            );
        }

        RegisterResponse {
            result: Some(register_response::Result::Success(ok)),
        }
        .encode_to_vec()
    }

    /// 测试：首次注册（无 PSK）→ AIS 返回 credential + PSK → PSK 被存储
    #[tokio::test]
    async fn bootstrap_first_registration_stores_psk() {
        let response_body = fake_register_response_bytes(true);

        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/register")
            .with_status(200)
            .with_header("content-type", "application/x-protobuf")
            .with_body(response_body)
            .create_async()
            .await;

        let dir = TempDir::new().unwrap();
        let config = dev_config(&dir);
        let hyper = Hyper::init(config).await.unwrap();

        let manifest = fake_manifest();
        let result = hyper
            .bootstrap_credential(&manifest, &server.url(), 1)
            .await;

        mock.assert_async().await;
        assert!(result.is_ok(), "首次注册应成功，错误: {:?}", result.err());

        // 验证 PSK 已被写入 ActorStore
        let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
        let store = ActorStore::open(&storage_path).await.unwrap();
        let psk = store.kv_get("hyper:psk:token").await.unwrap();
        assert!(psk.is_some(), "首次注册后 PSK 应已存储到 ActorStore");
        assert_eq!(psk.unwrap(), b"fresh-psk-from-ais".to_vec());
    }

    /// 测试：有有效 PSK → 跳过 manifest 注册，直接走 PSK 续期路径
    #[tokio::test]
    async fn bootstrap_psk_renewal_skips_manifest() {
        let response_body = fake_register_response_bytes(false);

        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/register")
            .with_status(200)
            .with_header("content-type", "application/x-protobuf")
            .with_body(response_body)
            .expect(1) // 应只调用一次 /register
            .create_async()
            .await;

        let dir = TempDir::new().unwrap();
        let config = dev_config(&dir);
        let hyper = Hyper::init(config).await.unwrap();

        // 预先在 ActorStore 中写入有效 PSK
        let manifest = fake_manifest();
        let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
        let store = ActorStore::open(&storage_path).await.unwrap();

        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        store
            .kv_set("hyper:psk:token", b"existing-valid-psk")
            .await
            .unwrap();
        store
            .kv_set("hyper:psk:expires_at", &expires_at.to_le_bytes())
            .await
            .unwrap();

        let result = hyper
            .bootstrap_credential(&manifest, &server.url(), 1)
            .await;

        mock.assert_async().await;
        assert!(result.is_ok(), "PSK 续期应成功，错误: {:?}", result.err());
    }

    /// 测试：PSK 过期 → 重走 manifest 注册路径
    #[tokio::test]
    async fn bootstrap_expired_psk_falls_back_to_manifest() {
        let response_body = fake_register_response_bytes(true);

        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/register")
            .with_status(200)
            .with_header("content-type", "application/x-protobuf")
            .with_body(response_body)
            .expect(1)
            .create_async()
            .await;

        let dir = TempDir::new().unwrap();
        let config = dev_config(&dir);
        let hyper = Hyper::init(config).await.unwrap();

        // 预先写入已过期的 PSK
        let manifest = fake_manifest();
        let storage_path = hyper.resolve_storage_path(&manifest).unwrap();
        let store = ActorStore::open(&storage_path).await.unwrap();

        let expired_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(10); // 10秒前已过期
        store
            .kv_set("hyper:psk:token", b"expired-psk")
            .await
            .unwrap();
        store
            .kv_set("hyper:psk:expires_at", &expired_at.to_le_bytes())
            .await
            .unwrap();

        let result = hyper
            .bootstrap_credential(&manifest, &server.url(), 1)
            .await;

        mock.assert_async().await;
        assert!(
            result.is_ok(),
            "PSK 过期后走 manifest 路径应成功，错误: {:?}",
            result.err()
        );
    }

    /// 测试：AIS 返回错误时正确传播 HyperError::AisBootstrapFailed
    #[tokio::test]
    async fn bootstrap_ais_error_propagates() {
        use actr_protocol::{ErrorResponse, RegisterResponse, register_response};

        let error_resp = RegisterResponse {
            result: Some(register_response::Result::Error(ErrorResponse {
                code: 403,
                message: "manufacturer not trusted".to_string(),
            })),
        }
        .encode_to_vec();

        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/register")
            .with_status(200)
            .with_header("content-type", "application/x-protobuf")
            .with_body(error_resp)
            .create_async()
            .await;

        let dir = TempDir::new().unwrap();
        let config = dev_config(&dir);
        let hyper = Hyper::init(config).await.unwrap();

        let manifest = fake_manifest();
        let result = hyper
            .bootstrap_credential(&manifest, &server.url(), 1)
            .await;

        assert!(
            matches!(result, Err(HyperError::AisBootstrapFailed(_))),
            "AIS 返回错误时应传播 AisBootstrapFailed，实际: {:?}",
            result
        );
    }
}
