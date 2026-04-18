use serde::{Deserialize, Serialize};

/// Package manifest, parsed from manifest.toml inside .actr package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
    pub manufacturer: String,
    pub name: String,
    pub version: String,
    pub binary: BinaryEntry,
    #[serde(default = "default_sig_algorithm")]
    pub signature_algorithm: String,
    /// ID of the public key used for signing (allows key rotation and lookup of historical keys).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_key_id: Option<String>,
    #[serde(default)]
    pub resources: Vec<ResourceEntry>,
    /// Proto files included in the package for service API definition.
    #[serde(default)]
    pub proto_files: Vec<ProtoFileEntry>,
    /// Optional workload dependency lock file packaged with the workload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_file: Option<LockFileEntry>,
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

impl BinaryEntry {
    /// Decode the hex-encoded SHA-256 hash string into a 32-byte array.
    ///
    /// Returns [`PackError::ManifestParseError`] if the stored `hash` is not a
    /// 64-character hex string.
    pub fn hash_bytes(&self) -> Result<[u8; 32], crate::error::PackError> {
        let hex = &self.hash;
        if hex.len() != 64 {
            return Err(crate::error::PackError::ManifestParseError(
                "binary.hash must be a 64-character hex string (32 bytes)".to_string(),
            ));
        }
        let mut out = [0u8; 32];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let s = std::str::from_utf8(chunk).map_err(|_| {
                crate::error::PackError::ManifestParseError(
                    "binary.hash contains non-UTF-8 characters".to_string(),
                )
            })?;
            out[i] = u8::from_str_radix(s, 16).map_err(|_| {
                crate::error::PackError::ManifestParseError(
                    "binary.hash contains invalid hex characters".to_string(),
                )
            })?;
        }
        Ok(out)
    }

    /// Returns `true` when this binary targets a WASM runtime (e.g.
    /// `wasm32-wasip1`, `wasm32-unknown-unknown`).
    pub fn is_wasm_target(&self) -> bool {
        self.target.starts_with("wasm32-")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceEntry {
    pub path: String,
    /// SHA-256 hash hex string (64 chars)
    pub hash: String,
}

/// Entry for a proto file included in the .actr package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtoFileEntry {
    /// File name (e.g. "echo.proto")
    pub name: String,
    /// Path inside the ZIP (e.g. "proto/echo.proto")
    pub path: String,
    /// SHA-256 hash hex string (64 chars)
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFileEntry {
    /// Path inside the ZIP (always `manifest.lock.toml` for packaged workloads).
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
