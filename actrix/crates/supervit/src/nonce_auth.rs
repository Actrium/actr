//! nonce-auth 集成模块
//!
//! 提供 nonce-auth 与 gRPC protobuf 之间的转换和封装

use crate::error::{Result, SupervitError};
use ::nonce_auth::{CredentialBuilder, NonceCredential};
use actrix_proto::NonceCredential as ProtoCredential;

/// 将 nonce-auth 凭证转换为 protobuf 格式
///
/// # Arguments
///
/// * `credential` - nonce-auth 生成的凭证
///
/// # Returns
///
/// protobuf 格式的凭证，可直接嵌入 gRPC 消息
pub fn to_proto_credential(credential: NonceCredential) -> ProtoCredential {
    ProtoCredential {
        timestamp: credential.timestamp,
        nonce: credential.nonce,
        signature: credential.signature,
    }
}

/// 将 protobuf 凭证转换为 nonce-auth 格式
///
/// # Arguments
///
/// * `proto` - protobuf 格式的凭证
///
/// # Returns
///
/// nonce-auth 格式的凭证，用于服务端验证
pub fn from_proto_credential(proto: &ProtoCredential) -> NonceCredential {
    NonceCredential {
        timestamp: proto.timestamp,
        nonce: proto.nonce.clone(),
        signature: proto.signature.clone(),
    }
}

/// 为请求生成认证凭证
///
/// # Arguments
///
/// * `shared_secret` - 共享密钥（hex 解码后的字节）
/// * `payload` - 请求负载（用于签名）
///
/// # Returns
///
/// protobuf 格式的认证凭证
///
/// # Example
///
/// ```ignore
/// let secret = hex::decode("abc123...").unwrap();
/// let payload = format!("health_check:{}", node_id);
/// let credential = generate_credential(&secret, payload.as_bytes())?;
/// ```
pub fn generate_credential(shared_secret: &[u8], payload: &[u8]) -> Result<ProtoCredential> {
    let credential = CredentialBuilder::new(shared_secret)
        .sign(payload)
        .map_err(|e| {
            SupervitError::Authentication(format!("Failed to generate credential: {e}"))
        })?;

    Ok(to_proto_credential(credential))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_conversion() {
        let original = NonceCredential {
            timestamp: 1699999999,
            nonce: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            signature: "YWJjMTIz".to_string(),
        };

        let proto = to_proto_credential(original.clone());
        let converted = from_proto_credential(&proto);

        assert_eq!(original.timestamp, converted.timestamp);
        assert_eq!(original.nonce, converted.nonce);
        assert_eq!(original.signature, converted.signature);
    }

    #[test]
    fn test_generate_credential() {
        let secret = b"test-shared-secret-key-32-bytes!";
        let payload = b"health_check:test-node";

        let result = generate_credential(secret, payload);
        assert!(result.is_ok());

        let credential = result.unwrap();
        assert!(credential.timestamp > 0);
        assert!(!credential.nonce.is_empty());
        assert!(!credential.signature.is_empty());
    }

    #[test]
    fn test_generate_credential_different_payloads() {
        let secret = b"test-shared-secret-key-32-bytes!";
        let payload1 = b"health_check:node1";
        let payload2 = b"health_check:node2";

        let cred1 = generate_credential(secret, payload1).unwrap();
        let cred2 = generate_credential(secret, payload2).unwrap();

        // 不同的 payload 应该产生不同的签名
        assert_ne!(cred1.signature, cred2.signature);
        // 但 nonce 应该都是唯一的
        assert_ne!(cred1.nonce, cred2.nonce);
    }
}
