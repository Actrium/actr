//! Plain TURN token payload.
//!
//! The token is serialized into JSON and embedded inside TURN username claims.

use crate::ActrId;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Plain-text TURN token containing identity context and the pre-shared key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    /// Optional expiration timestamp (seconds since Unix epoch).
    pub exp: Option<u64>,

    /// Realm identifier.
    pub realm_id: u32,

    /// Optional actor identifier tied to this credential.
    pub id: Option<ActrId>,

    /// Actor type string.
    pub act_type: String,

    /// Pre-shared key in hex encoding.
    pub psk: String,

    /// Optional device fingerprint provided by the client.
    pub device_fingerprint: Option<String>,
}

impl Token {
    /// Create a new TURN token with the required fields.
    pub fn new(realm_id: u32, act_type: String, psk: String, exp: Option<u64>) -> Self {
        Self {
            exp,
            realm_id,
            id: None,
            act_type,
            psk,
            device_fingerprint: None,
        }
    }

    /// Check whether the token has expired.
    ///
    /// A missing expiration means the token is treated as non-expiring.
    pub fn is_expired(&self) -> bool {
        if let Some(exp) = self.exp {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            now > exp
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn creates_token_with_expected_fields() {
        let token = Token::new(
            42,
            "user".to_string(),
            "abc123".to_string(),
            Some(1730614800),
        );

        assert_eq!(token.realm_id, 42);
        assert_eq!(token.act_type, "user");
        assert_eq!(token.psk, "abc123");
        assert_eq!(token.exp, Some(1730614800));
        assert!(token.id.is_none());
        assert!(token.device_fingerprint.is_none());
    }

    #[test]
    fn reports_expiration_correctly() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let valid_token = Token::new(7, "user".to_string(), "psk".to_string(), Some(now + 3600));
        assert!(!valid_token.is_expired());

        let expired_token = Token::new(7, "user".to_string(), "psk".to_string(), Some(now - 1));
        assert!(expired_token.is_expired());

        let no_exp_token = Token::new(7, "user".to_string(), "psk".to_string(), None);
        assert!(!no_exp_token.is_expired());
    }
}
