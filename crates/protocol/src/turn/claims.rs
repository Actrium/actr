//! TURN username claims.
//!
//! Claims are serialized into the TURN username and carry a base64-encoded,
//! JSON-formatted [`Token`] payload.

use crate::turn::token::Token;
use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// TURN authentication claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Tenant identifier.
    pub tenant_id: u32,

    /// Key identifier used by the TURN auth cache.
    pub key_id: u32,

    /// Base64-encoded token bytes.
    #[serde(
        serialize_with = "serialize_base64",
        deserialize_with = "deserialize_base64"
    )]
    pub token: Vec<u8>,
}

/// Serialize raw bytes into a base64 string.
fn serialize_base64<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&STANDARD.encode(bytes))
}

/// Deserialize a base64 string back into raw bytes.
fn deserialize_base64<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    STANDARD.decode(s).map_err(serde::de::Error::custom)
}

impl Claims {
    /// Create a new claims payload.
    pub fn new(tenant_id: u32, key_id: u32, token: Vec<u8>) -> Self {
        Self {
            tenant_id,
            key_id,
            token,
        }
    }

    /// Decode the embedded token payload.
    ///
    /// The token is kept as plain JSON to stay within TURN username limits. TLS
    /// on the TURN channel provides transport-level protection.
    pub fn get_token(&self) -> Result<Token> {
        let token: Token = serde_json::from_slice(&self.token)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize token: {e}"))?;

        if token.is_expired() {
            return Err(anyhow::anyhow!("Token has expired"));
        }

        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_claims() {
        let token_bytes = b"test_token".to_vec();
        let claims = Claims::new(123, 456, token_bytes.clone());

        assert_eq!(claims.tenant_id, 123);
        assert_eq!(claims.key_id, 456);
        assert_eq!(claims.token, token_bytes);
    }

    #[test]
    fn serializes_and_deserializes() {
        let token_bytes = b"test_token".to_vec();
        let claims = Claims::new(123, 456, token_bytes);

        let json = serde_json::to_string(&claims).unwrap();
        let decoded: Claims = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.tenant_id, claims.tenant_id);
        assert_eq!(decoded.key_id, claims.key_id);
        assert_eq!(decoded.token, claims.token);
    }

    #[test]
    fn encodes_token_as_base64() {
        let token_bytes = vec![0x01, 0x02, 0x03, 0x04];
        let claims = Claims::new(1, 1, token_bytes.clone());

        let json = serde_json::to_string(&claims).unwrap();
        let expected_base64 = STANDARD.encode(&token_bytes);

        assert!(json.contains(&expected_base64));
    }
}
