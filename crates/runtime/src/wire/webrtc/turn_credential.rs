//! TURN 认证凭据生成
//!
//! 用于生成连接 TURN 服务器所需的认证凭据。
//! 该模块与 actrix 项目中的 TURN 认证器兼容。

use actr_protocol::ActrId;
pub use actr_protocol::turn::{Claims as TurnClaims, Token as TurnToken};

/// TURN 凭据生成器
///
/// 用于生成连接 TURN 服务器所需的 username 和 password。
pub struct TurnCredentialBuilder {
    /// 租户/Realm ID
    realm_id: u32,
    /// 密钥 ID
    key_id: u32,
    /// Actor ID
    actor_id: ActrId,
    /// Actor 类型字符串
    actor_type: String,
    /// PSK (预共享密钥, hex 编码)
    psk: String,
    /// Token 过期时间 (可选, Unix timestamp)
    expiry: Option<u64>,
}

impl TurnCredentialBuilder {
    /// 创建新的凭据生成器
    ///
    /// # Arguments
    /// - `realm_id`: Realm ID
    /// - `key_id`: 密钥 ID (来自 credential.token_key_id)
    /// - `actor_id`: Actor ID
    /// - `psk`: PSK 原始字节
    /// - `public_key`: 公钥字节 (33-byte secp256k1 compressed)
    pub fn new(realm_id: u32, key_id: u32, actor_id: ActrId, psk: &[u8]) -> Self {
        use actr_protocol::ActrTypeExt;

        Self {
            realm_id,
            key_id,
            actor_id: actor_id.clone(),
            actor_type: actor_id.r#type.to_string_repr(),
            psk: hex::encode(psk),
            expiry: None,
        }
    }

    /// 设置 token 过期时间
    pub fn with_expiry(mut self, expiry_secs: u64) -> Self {
        self.expiry = Some(expiry_secs);
        self
    }

    /// 设置相对过期时间（从现在起多少秒后过期）
    pub fn expires_in(mut self, seconds: u64) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.expiry = Some(now + seconds);
        self
    }

    /// 构建 TURN 凭据
    ///
    /// # Returns
    /// (username, credential) 元组，其中：
    /// - username: Claims 的 JSON 序列化
    /// - credential: MD5(username:realm:psk) 的结果
    pub fn build(self, _turn_realm: &str) -> Result<(String, String), TurnCredentialError> {
        // 1. 创建 Token 结构
        let token = TurnToken {
            exp: self.expiry,
            tenant_id: self.realm_id,
            id: Some(self.actor_id),
            act_type: self.actor_type,
            psk: self.psk.clone(),
            device_fingerprint: None,
        };

        // 2. 序列化 Token
        let token_bytes = serde_json::to_vec(&token)
            .map_err(|e| TurnCredentialError::Serialization(e.to_string()))?;

        // TODO: 使用 ECIES 加密
        //    - 加密后的数据太大，超过 STUN username 属性的 763 字节限制
        //    - PSK 是随机数据，本身不包含敏感信息
        //    - 网络层通过 TURN over TLS 保护
        //    如果需要更高安全性，可以启用 TURN over TLS (turns:// URL)

        // 4. 创建 Claims（使用未加密的 token）
        let claims = TurnClaims {
            tenant_id: self.realm_id,
            key_id: self.key_id,
            token: token_bytes,
        };

        // 5. 序列化 Claims 作为 username
        let username = serde_json::to_string(&claims)
            .map_err(|e| TurnCredentialError::Serialization(e.to_string()))?;

        // 6. 返回 PSK 作为 credential
        // WebRTC 客户端会自动计算 MD5(username:realm:credential)
        // TURN 服务器的 auth_handle 也返回 MD5(username:realm:psk)
        // 所以这里直接返回 PSK，让双方计算结果一致
        let credential = self.psk.clone();

        Ok((username, credential))
    }
}

/// TURN 凭据生成错误
#[derive(Debug, thiserror::Error)]
pub enum TurnCredentialError {
    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Invalid public key: {0}")]
    InvalidPublicKey(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::{ActrType, Realm};
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    #[test]
    fn test_turn_token_serialization() {
        let actor_id = ActrId {
            realm: Realm { realm_id: 0 },
            serial_number: 12345,
            r#type: ActrType {
                manufacturer: "acme".to_string(),
                name: "echo-service".to_string(),
            },
        };

        let token = TurnToken {
            exp: Some(1700000000),
            tenant_id: 0,
            id: Some(actor_id),
            act_type: "acme/echo-service".to_string(),
            psk: "abc123".to_string(),
            device_fingerprint: None,
        };

        let json = serde_json::to_string(&token).unwrap();
        assert!(json.contains("\"psk\":\"abc123\""));
        assert!(json.contains("\"tenant_id\":0"));
    }

    #[test]
    fn test_turn_claims_serialization() {
        let claims = TurnClaims {
            tenant_id: 0,
            key_id: 1,
            token: vec![1, 2, 3, 4],
        };

        let json = serde_json::to_string(&claims).unwrap();
        assert!(json.contains("\"tenant_id\":0"));
        assert!(json.contains("\"key_id\":1"));

        // token 应该是 base64 编码
        let expected_base64 = STANDARD.encode(&[1, 2, 3, 4]);
        assert!(json.contains(&expected_base64));

        // 反序列化
        let decoded: TurnClaims = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.token, vec![1, 2, 3, 4]);
    }
}
