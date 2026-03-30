use thiserror::Error;

#[derive(Debug, Error)]
pub enum PackError {
    #[error("invalid package: {0}")]
    InvalidPackage(String),

    #[error("manifest.toml not found in package")]
    ManifestNotFound,

    #[error("manifest parse error: {0}")]
    ManifestParseError(String),

    #[error("manifest.sig not found in package")]
    SignatureNotFound,

    #[error("binary not found in package: {0}")]
    BinaryNotFound(String),

    #[error("binary hash mismatch: {path}")]
    BinaryHashMismatch { path: String },

    #[error("resource hash mismatch: {path}")]
    ResourceHashMismatch { path: String },

    #[error("proto file hash mismatch: {path}")]
    ProtoHashMismatch { path: String },

    #[error("manifest lock file hash mismatch: {path}")]
    LockFileHashMismatch { path: String },

    #[error("signature verification failed: {0}")]
    SignatureVerificationFailed(String),

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("ZIP error: {0}")]
    ZipError(#[from] zip::result::ZipError),
}
