use serde::{Deserialize, Serialize};

/// Package manifest, parsed from actr.toml inside .actr package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
    pub manufacturer: String,
    pub name: String,
    pub version: String,
    pub binary: BinaryEntry,
    #[serde(default = "default_sig_algorithm")]
    pub signature_algorithm: String,
    #[serde(default)]
    pub resources: Vec<ResourceEntry>,
    #[serde(default)]
    pub metadata: ManifestMetadata,
}

fn default_sig_algorithm() -> String {
    "ed25519".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryEntry {
    pub path: String,
    pub target: String,
    /// SHA-256 hash hex string (64 chars)
    pub hash: String,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceEntry {
    pub path: String,
    /// SHA-256 hash hex string (64 chars)
    pub hash: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestMetadata {
    pub description: Option<String>,
    pub license: Option<String>,
}

impl PackageManifest {
    /// Full type string: manufacturer:name:version
    pub fn actr_type_str(&self) -> String {
        format!("{}:{}:{}", self.manufacturer, self.name, self.version)
    }

    /// Parse from TOML string
    pub fn from_toml(s: &str) -> Result<Self, crate::error::PackError> {
        toml::from_str(s).map_err(|e| crate::error::PackError::ManifestParseError(e.to_string()))
    }

    /// Serialize to TOML string
    pub fn to_toml(&self) -> Result<String, crate::error::PackError> {
        toml::to_string_pretty(self)
            .map_err(|e| crate::error::PackError::ManifestParseError(e.to_string()))
    }
}
