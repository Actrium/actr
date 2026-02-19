//! KS 密钥加密模块
//!
//! 使用 AES-256-GCM 对存储的私钥进行加密保护

// Allow deprecated generic-array::from_slice until aes-gcm upgrades
#![allow(deprecated)]

use crate::error::{KsError, KsResult};
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, OsRng},
};
use base64::prelude::*;
use rand::RngCore;
use tracing::{debug, info};

/// KEK (Key Encryption Key) 来源
#[derive(Debug, Clone)]
pub enum KekSource {
    /// 直接从配置文件读取 KEK
    Direct(String),
    /// 从环境变量读取 KEK
    Environment(String),
    /// 从文件路径读取 KEK
    File(String),
}

/// 密钥加密器
///
/// 使用 AES-256-GCM 对私钥进行加密/解密
/// 加密格式: base64(nonce[12] || ciphertext || tag[16])
#[derive(Clone)]
pub struct KeyEncryptor {
    cipher: Option<Aes256Gcm>,
}

impl std::fmt::Debug for KeyEncryptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyEncryptor")
            .field("encryption_enabled", &self.is_enabled())
            .finish()
    }
}

impl KeyEncryptor {
    /// 创建无加密的加密器（兼容模式）
    pub fn no_encryption() -> Self {
        debug!("Creating KeyEncryptor in no-encryption mode");
        Self { cipher: None }
    }

    /// 从 KEK 源创建加密器
    pub fn from_kek_source(source: &KekSource) -> KsResult<Self> {
        let kek = match source {
            KekSource::Direct(key) => {
                debug!("Loading KEK from direct configuration");
                key.clone()
            }
            KekSource::Environment(env_var) => {
                debug!("Loading KEK from environment variable: {}", env_var);
                std::env::var(env_var).map_err(|e| {
                    KsError::Config(format!(
                        "Failed to read KEK from environment variable {env_var}: {e}"
                    ))
                })?
            }
            KekSource::File(path) => {
                debug!("Loading KEK from file: {}", path);
                std::fs::read_to_string(path).map_err(|e| {
                    KsError::Config(format!("Failed to read KEK from file {path}: {e}"))
                })?
            }
        };

        Self::from_kek(&kek)
    }

    /// 从 KEK 字符串创建加密器
    ///
    /// KEK 可以是:
    /// - 64 字符的十六进制字符串 (32 字节)
    /// - 44 字符的 Base64 字符串 (32 字节)
    pub fn from_kek(kek: &str) -> KsResult<Self> {
        let kek = kek.trim();

        // 尝试解析为十六进制
        let key_bytes = if kek.len() == 64 {
            hex::decode(kek).map_err(|e| KsError::Config(format!("Invalid KEK hex format: {e}")))?
        } else if kek.len() == 44 || kek.len() == 43 {
            // Base64 编码的 32 字节密钥
            BASE64_STANDARD
                .decode(kek)
                .map_err(|e| KsError::Config(format!("Invalid KEK base64 format: {e}")))?
        } else {
            return Err(KsError::Config(format!(
                "Invalid KEK length: expected 64 hex chars or 44 base64 chars, got {}",
                kek.len()
            )));
        };

        if key_bytes.len() != 32 {
            return Err(KsError::Config(format!(
                "Invalid KEK size: expected 32 bytes, got {}",
                key_bytes.len()
            )));
        }

        let cipher = Aes256Gcm::new_from_slice(&key_bytes)
            .map_err(|e| KsError::Crypto(format!("Failed to create cipher: {e}")))?;

        info!("KEK loaded successfully");
        Ok(Self {
            cipher: Some(cipher),
        })
    }

    /// 加密私钥
    ///
    /// 如果未配置 KEK，直接返回原始私钥（向后兼容）
    /// 否则使用 AES-256-GCM 加密
    ///
    /// 加密格式: base64(nonce[12] || ciphertext || tag[16])
    pub fn encrypt(&self, secret_key: &str) -> KsResult<String> {
        let cipher = match &self.cipher {
            Some(c) => c,
            None => {
                // 无加密模式，直接返回原文
                return Ok(secret_key.to_string());
            }
        };

        // 生成随机 nonce (12 字节)
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // 加密
        let ciphertext = cipher
            .encrypt(nonce, secret_key.as_bytes())
            .map_err(|e| KsError::Crypto(format!("Encryption failed: {e}")))?;

        // 组合: nonce || ciphertext (包含 tag)
        let mut encrypted = Vec::with_capacity(12 + ciphertext.len());
        encrypted.extend_from_slice(&nonce_bytes);
        encrypted.extend_from_slice(&ciphertext);

        // Base64 编码
        Ok(BASE64_STANDARD.encode(&encrypted))
    }

    /// 解密私钥
    ///
    /// 如果未配置 KEK，直接返回原始数据（向后兼容）
    /// 否则尝试 Base64 解码并使用 AES-256-GCM 解密
    pub fn decrypt(&self, encrypted_key: &str) -> KsResult<String> {
        let cipher = match &self.cipher {
            Some(c) => c,
            None => {
                // 无加密模式，直接返回原文
                return Ok(encrypted_key.to_string());
            }
        };

        // Base64 解码
        let encrypted_bytes = BASE64_STANDARD
            .decode(encrypted_key)
            .map_err(|e| KsError::Crypto(format!("Invalid encrypted key format: {e}")))?;

        if encrypted_bytes.len() < 12 + 16 {
            return Err(KsError::Crypto(format!(
                "Invalid encrypted key size: expected at least 28 bytes, got {}",
                encrypted_bytes.len()
            )));
        }

        // 分离 nonce 和 ciphertext
        let (nonce_bytes, ciphertext) = encrypted_bytes.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        // 解密
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| KsError::Crypto(format!("Decryption failed: {e}")))?;

        String::from_utf8(plaintext)
            .map_err(|e| KsError::Crypto(format!("Invalid UTF-8 after decryption: {e}")))
    }

    /// 是否启用了加密
    pub fn is_enabled(&self) -> bool {
        self.cipher.is_some()
    }

    /// 生成新的 KEK（用于初始化）
    ///
    /// 返回十六进制格式的 32 字节随机密钥
    pub fn generate_kek() -> String {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        hex::encode(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_encryption_mode() {
        let encryptor = KeyEncryptor::no_encryption();
        assert!(!encryptor.is_enabled());

        let original = "secret-key-data";
        let encrypted = encryptor.encrypt(original).unwrap();
        assert_eq!(encrypted, original);

        let decrypted = encryptor.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, original);
    }

    #[test]
    fn test_encryption_decryption_hex_kek() {
        let kek = KeyEncryptor::generate_kek();
        let encryptor = KeyEncryptor::from_kek(&kek).unwrap();
        assert!(encryptor.is_enabled());

        let original = "my-secret-private-key-base64-encoded";
        let encrypted = encryptor.encrypt(original).unwrap();

        // 加密后应该不同
        assert_ne!(encrypted, original);

        // 解密后应该相同
        let decrypted = encryptor.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, original);
    }

    #[test]
    fn test_encryption_decryption_base64_kek() {
        // 生成 32 字节密钥并编码为 Base64
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let kek_b64 = BASE64_STANDARD.encode(key);

        let encryptor = KeyEncryptor::from_kek(&kek_b64).unwrap();
        assert!(encryptor.is_enabled());

        let original = "another-secret-key";
        let encrypted = encryptor.encrypt(original).unwrap();
        let decrypted = encryptor.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, original);
    }

    #[test]
    fn test_invalid_kek_length() {
        let result = KeyEncryptor::from_kek("too-short");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid KEK length")
        );
    }

    #[test]
    fn test_invalid_kek_hex() {
        let invalid_hex = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
        let result = KeyEncryptor::from_kek(invalid_hex);
        assert!(result.is_err());
    }

    #[test]
    fn test_decrypt_with_wrong_kek() {
        let kek1 = KeyEncryptor::generate_kek();
        let encryptor1 = KeyEncryptor::from_kek(&kek1).unwrap();

        let original = "secret-data";
        let encrypted = encryptor1.encrypt(original).unwrap();

        // 用不同的 KEK 尝试解密
        let kek2 = KeyEncryptor::generate_kek();
        let encryptor2 = KeyEncryptor::from_kek(&kek2).unwrap();

        let result = encryptor2.decrypt(&encrypted);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Decryption failed")
        );
    }

    #[test]
    fn test_decrypt_invalid_base64() {
        let kek = KeyEncryptor::generate_kek();
        let encryptor = KeyEncryptor::from_kek(&kek).unwrap();

        let result = encryptor.decrypt("not-valid-base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn test_decrypt_too_short() {
        let kek = KeyEncryptor::generate_kek();
        let encryptor = KeyEncryptor::from_kek(&kek).unwrap();

        // 创建一个太短的加密数据（少于 28 字节）
        let short_data = BASE64_STANDARD.encode([0u8; 10]);
        let result = encryptor.decrypt(&short_data);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid encrypted key size")
        );
    }

    #[test]
    fn test_kek_from_environment() {
        let kek = KeyEncryptor::generate_kek();
        unsafe {
            std::env::set_var("TEST_KEK_ENV", &kek);
        }

        let encryptor =
            KeyEncryptor::from_kek_source(&KekSource::Environment("TEST_KEK_ENV".to_string()))
                .unwrap();
        assert!(encryptor.is_enabled());

        unsafe {
            std::env::remove_var("TEST_KEK_ENV");
        }
    }

    #[test]
    fn test_kek_from_file() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let kek = KeyEncryptor::generate_kek();
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(kek.as_bytes()).unwrap();
        file.flush().unwrap();

        let encryptor = KeyEncryptor::from_kek_source(&KekSource::File(
            file.path().to_string_lossy().to_string(),
        ))
        .unwrap();
        assert!(encryptor.is_enabled());
    }

    #[test]
    fn test_generate_kek_format() {
        let kek = KeyEncryptor::generate_kek();
        // 应该是 64 个十六进制字符
        assert_eq!(kek.len(), 64);
        // 应该可以成功创建加密器
        let encryptor = KeyEncryptor::from_kek(&kek).unwrap();
        assert!(encryptor.is_enabled());
    }
}
