//! # Actor-RTC URI parsing library
//!
//! Provides standard URI parsing for the actr:// protocol, without business logic.
//!
//! URI format: actr://<realm>:<manufacturer>+<name>@<version>
//! Example: actr://101:acme+echo-service@1.0.0

use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use thiserror::Error;

/// Actor-RTC URI parse error
#[derive(Error, Debug)]
pub enum ActrUriError {
    #[error("Invalid URI scheme, expected 'actr' but got '{0}'")]
    InvalidScheme(String),

    #[error("Missing actor authority in URI")]
    MissingAuthority,

    #[error("Invalid actor authority format, expected: <realm>:<manufacturer>+<name>@<version>")]
    InvalidAuthorityFormat(String),

    #[error("Missing version suffix '@<version>' in URI")]
    MissingVersion,

    #[error("Invalid realm ID: {0}")]
    InvalidRealmId(String),

    #[error("URI parse error: {0}")]
    ParseError(String),
}

/// Actor-RTC URI structure
/// Format: actr://<realm>:<manufacturer>+<name>@<version>
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActrUri {
    /// Realm ID (u32)
    pub realm: u32,
    /// Manufacturer name
    pub manufacturer: String,
    /// Actor type name
    pub name: String,
    /// Version (e.g., "1.0.0")
    pub version: String,
}

impl ActrUri {
    /// Create a new Actor-RTC URI
    pub fn new(realm: u32, manufacturer: String, name: String, version: String) -> Self {
        Self {
            realm,
            manufacturer,
            name,
            version,
        }
    }

    /// Get scheme info
    pub fn scheme(&self) -> &'static str {
        "actr"
    }

    /// Get actor type string representation (manufacturer+name)
    pub fn actor_type(&self) -> String {
        format!("{}+{}", self.manufacturer, self.name)
    }
}

impl Display for ActrUri {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "actr://{}:{}+{}@{}",
            self.realm, self.manufacturer, self.name, self.version
        )
    }
}

impl FromStr for ActrUri {
    type Err = ActrUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("actr://") {
            return Err(ActrUriError::InvalidScheme(
                s.split(':').next().unwrap_or("").to_string(),
            ));
        }

        let without_scheme = &s[7..];

        // Check for empty authority
        if without_scheme.is_empty() {
            return Err(ActrUriError::MissingAuthority);
        }

        // Check for version suffix '@'
        let (authority, version) = without_scheme
            .rsplit_once('@')
            .ok_or(ActrUriError::MissingVersion)?;

        let version = version.to_string();

        // Parse realm:manufacturer+name
        let (realm_str, type_part) = authority
            .split_once(':')
            .ok_or_else(|| ActrUriError::InvalidAuthorityFormat(authority.to_string()))?;

        let realm = realm_str
            .parse::<u32>()
            .map_err(|_| ActrUriError::InvalidRealmId(realm_str.to_string()))?;

        let (manufacturer, name) = type_part
            .split_once('+')
            .ok_or_else(|| ActrUriError::InvalidAuthorityFormat(authority.to_string()))?;

        Ok(ActrUri {
            realm,
            manufacturer: manufacturer.to_string(),
            name: name.to_string(),
            version,
        })
    }
}

/// Actor-RTC URI builder
#[derive(Debug)]
pub struct ActrUriBuilder {
    realm: Option<u32>,
    manufacturer: Option<String>,
    name: Option<String>,
    version: String,
}

impl Default for ActrUriBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ActrUriBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            realm: None,
            manufacturer: None,
            name: None,
            version: String::new(),
        }
    }

    /// Set Realm ID
    pub fn realm(mut self, realm: u32) -> Self {
        self.realm = Some(realm);
        self
    }

    /// Set Manufacturer
    pub fn manufacturer<S: Into<String>>(mut self, manufacturer: S) -> Self {
        self.manufacturer = Some(manufacturer.into());
        self
    }

    /// Set Actor type name
    pub fn name<S: Into<String>>(mut self, name: S) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set version
    pub fn version<S: Into<String>>(mut self, version: S) -> Self {
        self.version = version.into();
        self
    }

    /// Build the URI
    pub fn build(self) -> Result<ActrUri, ActrUriError> {
        let realm = self.realm.ok_or(ActrUriError::MissingAuthority)?;
        let manufacturer = self
            .manufacturer
            .ok_or(ActrUriError::InvalidAuthorityFormat(
                "missing manufacturer".to_string(),
            ))?;
        let name = self.name.ok_or(ActrUriError::InvalidAuthorityFormat(
            "missing name".to_string(),
        ))?;
        if self.version.is_empty() {
            return Err(ActrUriError::InvalidAuthorityFormat(
                "missing version".to_string(),
            ));
        }

        Ok(ActrUri {
            realm,
            manufacturer,
            name,
            version: self.version,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_uri_parsing() {
        let uri = "actr://101:acme+echo-service@1.0.0"
            .parse::<ActrUri>()
            .unwrap();
        assert_eq!(uri.realm, 101);
        assert_eq!(uri.manufacturer, "acme");
        assert_eq!(uri.name, "echo-service");
        assert_eq!(uri.version, "1.0.0");
    }

    #[test]
    fn test_uri_builder() {
        let uri = ActrUriBuilder::new()
            .realm(101)
            .manufacturer("acme")
            .name("order-service")
            .version("1.0.0")
            .build()
            .unwrap();

        assert_eq!(uri.realm, 101);
        assert_eq!(uri.manufacturer, "acme");
        assert_eq!(uri.name, "order-service");
        assert_eq!(uri.version, "1.0.0");
    }

    #[test]
    fn test_uri_builder_requires_version() {
        let result = ActrUriBuilder::new()
            .realm(101)
            .manufacturer("acme")
            .name("order-service")
            .build();

        assert!(matches!(
            result,
            Err(ActrUriError::InvalidAuthorityFormat(msg)) if msg == "missing version"
        ));
    }

    #[test]
    fn test_uri_to_string() {
        let uri = ActrUri::new(
            101,
            "acme".to_string(),
            "user-service".to_string(),
            "1.0.0".to_string(),
        );
        let uri_string = uri.to_string();
        assert_eq!(uri_string, "actr://101:acme+user-service@1.0.0");
    }

    #[test]
    fn test_invalid_scheme() {
        let result = "http://101:acme+service@1.0.0".parse::<ActrUri>();
        assert!(matches!(result, Err(ActrUriError::InvalidScheme(_))));
    }

    #[test]
    fn test_missing_authority() {
        let result = "actr://".parse::<ActrUri>();
        assert!(matches!(result, Err(ActrUriError::MissingAuthority)));
    }

    #[test]
    fn test_missing_version() {
        let result = "actr://101:acme+service".parse::<ActrUri>();
        assert!(matches!(result, Err(ActrUriError::MissingVersion)));
    }

    #[test]
    fn test_invalid_realm_id() {
        let result = "actr://abc:acme+service@1.0.0".parse::<ActrUri>();
        assert!(matches!(result, Err(ActrUriError::InvalidRealmId(_))));
    }

    #[test]
    fn test_invalid_authority_format() {
        let result = "actr://101:acme:service@1.0.0".parse::<ActrUri>();
        assert!(matches!(
            result,
            Err(ActrUriError::InvalidAuthorityFormat(_))
        ));
    }

    #[test]
    fn test_actor_type_method() {
        let uri = "actr://101:acme+user-service@1.0.0"
            .parse::<ActrUri>()
            .unwrap();
        assert_eq!(uri.actor_type(), "acme+user-service");
    }

    #[test]
    fn test_roundtrip() {
        let uri = ActrUriBuilder::new()
            .realm(9999)
            .manufacturer("test")
            .name("service")
            .version("1.0.0")
            .build()
            .unwrap();

        let uri_str = uri.to_string();
        let parsed = uri_str.parse::<ActrUri>().unwrap();
        assert_eq!(uri.realm, parsed.realm);
        assert_eq!(uri.manufacturer, parsed.manufacturer);
        assert_eq!(uri.name, parsed.name);
        assert_eq!(uri.version, parsed.version);
    }
}
