/// Signed manifest embedded in an Actr package.
///
/// Returned by the verification pipeline after validating an `.actr` package.
/// The MFR signs it at build time and Hyper verifies it before loading.
#[derive(Debug, Clone)]
pub struct PackageManifest {
    /// Actor manufacturer name, the `manufacturer` component of `manufacturer:name:version`.
    pub manufacturer: String,
    /// Actor name.
    pub actr_name: String,
    /// Actor version.
    pub version: String,
    /// SHA-256 hash of the binary, 32 bytes.
    pub binary_hash: [u8; 32],
    /// Capability list declared by the actor.
    pub capabilities: Vec<String>,
    /// Ed25519 signature over the manifest contents, excluding the `signature` field itself.
    pub signature: Vec<u8>,
}

impl PackageManifest {
    /// Full three-part `ActrType` string.
    pub fn actr_type_str(&self) -> String {
        format!("{}:{}:{}", self.manufacturer, self.actr_name, self.version)
    }
}
