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
    /// Binary path inside the `.actr` ZIP package.
    pub binary_path: String,
    /// Rust target triple recorded in the package manifest.
    pub binary_target: String,
    /// SHA-256 hash of the binary, 32 bytes.
    pub binary_hash: [u8; 32],
    /// Capability list declared by the actor.
    pub capabilities: Vec<String>,
    /// Ed25519 signature over the manifest contents, excluding the `signature` field itself.
    pub signature: Vec<u8>,
    /// Raw manifest bytes (original `actr.toml` from the `.actr` package).
    /// Preserved for transparent forwarding to AIS for signature verification.
    pub manifest_raw: Vec<u8>,
    /// Target platform (e.g. "wasm32-wasip1", "x86_64-unknown-linux-gnu").
    pub target: String,
}

impl PackageManifest {
    /// Full three-part `ActrType` string.
    pub fn actr_type_str(&self) -> String {
        format!("{}:{}:{}", self.manufacturer, self.actr_name, self.version)
    }

    /// Returns `true` when the package target should be executed by the WASM backend.
    pub fn is_wasm_target(&self) -> bool {
        self.binary_target.starts_with("wasm32-")
    }
}
