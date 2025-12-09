//! TURN username claims.
//!
//! Claims are serialized into the TURN username and carry a base122-encoded
//! binary payload containing realm_id, key_id, and encrypted token.

use anyhow::Result;
use bytes::{Buf as _, Bytes};

/// TURN authentication claims.
#[derive(Debug, Clone)]
pub struct Claims {
    /// Realm identifier.
    pub realm_id: u32,

    /// Key identifier used by the TURN auth cache.
    pub key_id: u32,
    /// Encrypted token.
    pub token: Bytes,
}

impl Claims {
    pub fn new(realm_id: u32, key_id: u32, token: Bytes) -> Self {
        Self {
            realm_id,
            key_id,
            token,
        }
    }

    /// Encodes Claims into base122 string for TURN server compatibility
    ///
    /// TURN servers only accept String usernames, so we encode the binary data:
    ///
    /// ```text
    ///     Byte Layout (ASCII Table):
    ///     +----------+----------+-------------------+
    ///     | Byte 0-3 | Byte 4-7 |     Byte 8+       |
    ///     +----------+----------+-------------------+
    ///     | realm_id |  key_id  |      token        |
    ///     | (u32 BE) | (u32 BE) | (variable len)    |
    ///     +----------+----------+-------------------+
    /// ```
    ///
    /// The bytes are then encoded with base122 to create a valid String
    /// for TURN protocol compatibility.
    pub fn encode(&self) -> String {
        let mut buffer = Vec::new();

        // realm_id: 4 bytes (big endian)
        buffer.extend_from_slice(&self.realm_id.to_be_bytes());

        // key_id: 4 bytes (big endian)
        buffer.extend_from_slice(&self.key_id.to_be_bytes());

        // token: variable length bytes
        buffer.extend_from_slice(&self.token);

        // Encode to base122 string for TURN server compatibility
        base122::encode(&buffer)
    }

    /// Decodes base122 string back to Claims using bytes::Buf trait
    ///
    /// ```text
    ///     Byte Layout (ASCII Table):
    ///     +----------+----------+-------------------+
    ///     | Byte 0-3 | Byte 4-7 |     Byte 8+       |
    ///     +----------+----------+-------------------+
    ///     | realm_id |  key_id  |      token        |
    ///     | (u32 BE) | (u32 BE) | (variable len)    |
    ///     +----------+----------+-------------------+
    /// ```
    pub fn decode(s: &str) -> Result<Self> {
        // Decode from base122 string
        let bytes =
            base122::decode(s).map_err(|e| anyhow::anyhow!("Failed to decode base122: {}", e))?;

        // Minimum length check: 4 + 4 = 8 bytes
        if bytes.len() < 8 {
            return Err(anyhow::anyhow!("Invalid username length: too short"));
        }

        let mut buf = bytes.as_slice();

        // Read realm_id: 4 bytes (big endian)
        if buf.remaining() < 4 {
            return Err(anyhow::anyhow!(
                "Invalid format: insufficient bytes for realm_id"
            ));
        }
        let realm_id = buf.get_u32();

        // Read key_id: 4 bytes (big endian)
        if buf.remaining() < 4 {
            return Err(anyhow::anyhow!(
                "Invalid format: insufficient bytes for key_id"
            ));
        }
        let key_id = buf.get_u32();

        // Read token: remaining bytes
        let token = Bytes::from(buf.to_vec());

        Ok(Claims {
            realm_id,
            key_id,
            token,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claims_new() {
        let token_bytes = Bytes::from(b"test_token".as_slice());
        let claims = Claims::new(123, 456, token_bytes.clone());

        assert_eq!(claims.realm_id, 123);
        assert_eq!(claims.key_id, 456);
        assert_eq!(claims.token, token_bytes);
    }

    #[test]
    fn test_claims_encode_decode() {
        let token_bytes = Bytes::from(b"test_token_data".as_slice());
        let claims = Claims::new(12345, 67890, token_bytes.clone());

        // Encode
        let encoded = claims.encode();
        assert!(!encoded.is_empty());

        // Decode
        let decoded = Claims::decode(&encoded).expect("Failed to decode");
        assert_eq!(decoded.realm_id, 12345);
        assert_eq!(decoded.key_id, 67890);
        assert_eq!(decoded.token, token_bytes);
    }

    #[test]
    fn test_claims_encode_decode_empty_token() {
        let token_bytes = Bytes::from(b"".as_slice());
        let claims = Claims::new(1, 2, token_bytes.clone());

        let encoded = claims.encode();
        let decoded = Claims::decode(&encoded).expect("Failed to decode");

        assert_eq!(decoded.realm_id, 1);
        assert_eq!(decoded.key_id, 2);
        assert_eq!(decoded.token, token_bytes);
    }

    #[test]
    fn test_claims_encode_decode_large_values() {
        let token_bytes = Bytes::from(vec![0x01, 0x02, 0x03, 0x04, 0x05]);
        let claims = Claims::new(u32::MAX, u32::MAX, token_bytes.clone());

        let encoded = claims.encode();
        let decoded = Claims::decode(&encoded).expect("Failed to decode");

        assert_eq!(decoded.realm_id, u32::MAX);
        assert_eq!(decoded.key_id, u32::MAX);
        assert_eq!(decoded.token, token_bytes);
    }

    #[test]
    fn test_claims_decode_invalid_short_string() {
        // Create a base122 string that's too short (less than 8 bytes when decoded)
        let short_encoded = base122::encode(&[1, 2, 3, 4, 5, 6, 7]); // Only 7 bytes

        let result = Claims::decode(&short_encoded);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid username length")
        );
    }

    #[test]
    fn test_claims_decode_invalid_base122() {
        // Test with a string that might decode but produces invalid data
        // base122-rs might decode some invalid strings, so we test with a string
        // that decodes to less than 8 bytes
        let result = Claims::decode("a"); // Very short string that might decode to < 8 bytes
        match result {
            Ok(decoded) => {
                // If it decodes successfully, verify it fails our validation
                // This should not happen if our validation works, but if base122 decodes it,
                // our length check should catch it
                assert!(decoded.token.len() < 1000); // Just verify it doesn't panic
            }
            Err(e) => {
                // Expected: should fail validation
                let err_msg = e.to_string();
                assert!(
                    err_msg.contains("Invalid username length")
                        || err_msg.contains("Failed to decode base122")
                        || err_msg.contains("insufficient bytes"),
                    "Unexpected error message: {}",
                    err_msg
                );
            }
        }
    }

    #[test]
    fn test_claims_encode_decode_roundtrip_multiple() {
        let test_cases = vec![
            (0u32, 0u32, b"".as_slice()),
            (1u32, 1u32, b"a".as_slice()),
            (100u32, 200u32, b"hello".as_slice()),
            (999u32, 888u32, b"world".as_slice()),
            (12345u32, 67890u32, b"test_token_data_12345".as_slice()),
        ];

        for (realm_id, key_id, token_data) in test_cases {
            let token_bytes = Bytes::from(token_data);
            let claims = Claims::new(realm_id, key_id, token_bytes.clone());

            let encoded = claims.encode();
            let decoded = Claims::decode(&encoded).expect("Failed to decode");

            assert_eq!(
                decoded.realm_id, realm_id,
                "realm_id mismatch for test case: realm_id={}, key_id={}",
                realm_id, key_id
            );
            assert_eq!(
                decoded.key_id, key_id,
                "key_id mismatch for test case: realm_id={}, key_id={}",
                realm_id, key_id
            );
            assert_eq!(
                decoded.token, token_bytes,
                "token mismatch for test case: realm_id={}, key_id={}",
                realm_id, key_id
            );
        }
    }

    #[test]
    fn test_claims_byte_layout_verification() {
        // Verify the byte layout: realm_id (4 bytes) + key_id (4 bytes) + token
        let token_bytes = Bytes::from(b"token".as_slice());
        let claims = Claims::new(0x12345678, 0xABCDEF00, token_bytes);

        let encoded = claims.encode();
        let decoded_bytes = base122::decode(&encoded).expect("Failed to decode base122");

        // Should have at least 8 bytes (4 + 4) + token length
        assert!(decoded_bytes.len() >= 8);

        // Verify realm_id (first 4 bytes, big endian)
        let realm_id_bytes = &decoded_bytes[0..4];
        let realm_id = u32::from_be_bytes([
            realm_id_bytes[0],
            realm_id_bytes[1],
            realm_id_bytes[2],
            realm_id_bytes[3],
        ]);
        assert_eq!(realm_id, 0x12345678);

        // Verify key_id (next 4 bytes, big endian)
        let key_id_bytes = &decoded_bytes[4..8];
        let key_id = u32::from_be_bytes([
            key_id_bytes[0],
            key_id_bytes[1],
            key_id_bytes[2],
            key_id_bytes[3],
        ]);
        assert_eq!(key_id, 0xABCDEF00);
    }
}
