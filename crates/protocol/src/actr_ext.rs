//! Actor identity formatting and parsing utilities
//!
//! This module provides string formatting and parsing helpers for `ActrType` and `ActrId`.
//! String forms are stable for logging, configuration, and CLI interactions.

use crate::{ActrId, ActrType, Realm, name::Name};
use std::str::FromStr;
use thiserror::Error;

/// Errors for actor identity parsing and formatting
#[derive(Error, Debug, PartialEq, Eq)]
pub enum ActrError {
    #[error(
        "Invalid Actor ID format: '{0}'. Expected format: <serial_number>@<realm_id>:<manufacturer>+<name>"
    )]
    InvalidFormat(String),

    #[error("Invalid component in actor identity: {0}")]
    InvalidComponent(String),

    #[error("Invalid actor type format: '{0}'. Expected format: <manufacturer>+<name>")]
    InvalidTypeFormat(String),

    /// 消息解码失败
    #[error("Failed to decode protobuf message: {message}")]
    DecodeFailure { message: String },

    /// 未知的路由键
    #[error("Unknown route key: {route_key}")]
    UnknownRoute { route_key: String },

    /// OutGate 尚未初始化
    #[error("Gate not initialized: {message}")]
    GateNotInitialized { message: String },

    /// Feature not yet implemented
    #[error("Feature not yet implemented: {feature}")]
    NotImplemented { feature: String },

    /// ACL 权限拒绝
    #[error("Permission denied: {message}")]
    PermissionDenied { message: String },

    /// 依赖未找到 - Actr.lock.toml 中不存在该依赖
    #[error("Dependency '{service_name}' not found: {message}")]
    DependencyNotFound {
        service_name: String,
        message: String,
    },
}

/// Helpers for `ActrType` string conversions
pub trait ActrTypeExt: Sized {
    /// Convert to stable string representation: "<manufacturer>+<name>".
    fn to_string_repr(&self) -> String;

    /// Parse from string representation. Performs validation on both parts.
    fn from_string_repr(s: &str) -> Result<Self, ActrError>;
}

impl ActrTypeExt for ActrType {
    fn to_string_repr(&self) -> String {
        format!("{}+{}", self.manufacturer, self.name)
    }

    fn from_string_repr(s: &str) -> Result<Self, ActrError> {
        let (manufacturer, name) = s
            .split_once('+')
            .ok_or_else(|| ActrError::InvalidTypeFormat(s.to_string()))?;

        // Reuse generic name validation to keep rules consistent across the project
        Name::new(manufacturer.to_string())
            .map_err(|e| ActrError::InvalidComponent(format!("Invalid manufacturer: {e}")))?;
        Name::new(name.to_string())
            .map_err(|e| ActrError::InvalidComponent(format!("Invalid type name: {e}")))?;

        Ok(ActrType {
            manufacturer: manufacturer.to_string(),
            name: name.to_string(),
        })
    }
}

/// Helpers for `ActrId` string conversions
pub trait ActrIdExt: Sized {
    /// Convert to "<serial_number_hex>@<realm_id>:<manufacturer>+<name>"
    fn to_string_repr(&self) -> String;

    /// Parse from string representation.
    fn from_string_repr(s: &str) -> Result<Self, ActrError>;
}

impl ActrIdExt for ActrId {
    fn to_string_repr(&self) -> String {
        format!(
            "{:x}@{}:{}+{}",
            self.serial_number, self.realm.realm_id, self.r#type.manufacturer, self.r#type.name
        )
    }

    fn from_string_repr(s: &str) -> Result<Self, ActrError> {
        let (serial_part, rest) = s
            .split_once('@')
            .ok_or_else(|| ActrError::InvalidFormat("Missing '@' separator".to_string()))?;

        let serial_number = u64::from_str_radix(serial_part, 16).map_err(|_e| {
            ActrError::InvalidComponent(format!("Invalid serial number hex: {serial_part}"))
        })?;

        let (realm_part, type_part) = rest
            .split_once(':')
            .ok_or_else(|| ActrError::InvalidFormat("Missing ':' separator".to_string()))?;

        let realm_id = u32::from_str(realm_part)
            .map_err(|_e| ActrError::InvalidComponent(format!("Invalid realm ID: {realm_part}")))?;

        let actr_type = ActrType::from_string_repr(type_part)?;

        Ok(ActrId {
            realm: Realm { realm_id },
            serial_number,
            r#type: actr_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_actor_id_string_repr_roundtrip() {
        let original_id = ActrId {
            realm: Realm { realm_id: 101 },
            serial_number: 0x1a2b3c,
            r#type: ActrType {
                manufacturer: "acme".to_string(),
                name: "echo-service".to_string(),
            },
        };

        let string_repr = original_id.to_string_repr();
        assert_eq!(string_repr, "1a2b3c@101:acme+echo-service");

        let parsed_id = ActrId::from_string_repr(&string_repr).unwrap();
        assert_eq!(original_id.realm.realm_id, parsed_id.realm.realm_id);
        assert_eq!(original_id.serial_number, parsed_id.serial_number);
        assert_eq!(
            original_id.r#type.manufacturer,
            parsed_id.r#type.manufacturer
        );
        assert_eq!(original_id.r#type.name, parsed_id.r#type.name);
    }

    #[test]
    fn test_invalid_format_parsing() {
        assert!(matches!(
            ActrId::from_string_repr("invalid-string"),
            Err(ActrError::InvalidFormat(_))
        ));
        assert!(matches!(
            ActrId::from_string_repr("123@101:acme"),
            Err(ActrError::InvalidTypeFormat(_))
        ));
        assert!(matches!(
            ActrId::from_string_repr("123@acme+echo"),
            Err(ActrError::InvalidFormat(_))
        ));
        assert!(matches!(
            ActrId::from_string_repr("xyz@101:acme+echo"),
            Err(ActrError::InvalidComponent(_))
        ));
    }

    #[test]
    fn test_actr_type_roundtrip_and_validation() {
        let s = "acme+echo";
        let ty = ActrType::from_string_repr(s).unwrap();
        assert_eq!(ty.to_string_repr(), s);

        // invalid format
        assert!(matches!(
            ActrType::from_string_repr("acme-echo"),
            Err(ActrError::InvalidTypeFormat(_))
        ));

        // invalid manufacturer via Name validation
        assert!(matches!(
            ActrType::from_string_repr("1acme+echo"),
            Err(ActrError::InvalidComponent(_))
        ));

        // invalid name via Name validation
        assert!(matches!(
            ActrType::from_string_repr("acme+echo!"),
            Err(ActrError::InvalidComponent(_))
        ));
    }
}
