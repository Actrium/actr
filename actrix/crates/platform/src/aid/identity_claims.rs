//! Identity claims used by AIS credentials.

use serde::{Deserialize, Serialize};

/// Identity claims signed into an AIdCredential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityClaims {
    /// Realm ID.
    pub realm_id: u32,

    /// Actor ID string representation.
    pub actor_id: String,

    /// Token expiration time as Unix timestamp seconds.
    pub expr_time: u64,
}

impl IdentityClaims {
    /// Create identity claims.
    pub fn new(realm_id: u32, actor_id: String, expr_time: u64) -> Self {
        Self {
            realm_id,
            actor_id,
            expr_time,
        }
    }

    /// Create identity claims from an ActrId.
    pub fn from_actr_id(actr_id: &actr_protocol::ActrId, expr_time: u64) -> Self {
        Self {
            realm_id: actr_id.realm.realm_id,
            actor_id: actr_id.to_string_repr(),
            expr_time,
        }
    }

    /// Check whether the token is expired.
    pub fn is_expired(&self) -> bool {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now > self.expr_time
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_identity_claims_creation() {
        let claims =
            IdentityClaims::new(12345, "1a2b3c@12345/apple:user:1".to_string(), 1730614800);

        assert_eq!(claims.realm_id, 12345);
        assert_eq!(claims.actor_id, "1a2b3c@12345/apple:user:1");
        assert_eq!(claims.expr_time, 1730614800);
    }

    #[test]
    fn test_identity_claims_expiration() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Not expired.
        let valid_claims = IdentityClaims::new(1, "123@1/acme:test:1".to_string(), now + 3600);
        assert!(!valid_claims.is_expired());

        // Expired.
        let expired_claims = IdentityClaims::new(1, "123@1/acme:test:1".to_string(), now - 1);
        assert!(expired_claims.is_expired());
    }
}
