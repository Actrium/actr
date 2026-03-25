//! actr-pack -- .actr package format
//!
//! Provides reading, writing, signing and verification of .actr ZIP STORE packages.
//!
//! ## Package structure
//!
//! ```text
//! {mfr}-{name}-{version}-{target}.actr
//! +-- actr.toml           # manifest (TOML)
//! +-- actr.sig            # Ed25519 signature (64 bytes raw)
//! +-- bin/actor.wasm      # binary (STORE mode, uncompressed)
//! ```
//!
//! ## Signing chain
//!
//! ```text
//! binary bytes -> SHA-256 -> actr.toml[binary.hash]
//!                                  |
//!                          actr.toml bytes -> Ed25519 sign -> actr.sig
//! ```

pub mod error;
pub mod load;
pub mod manifest;
pub mod pack;
pub mod verify;

pub use error::PackError;
pub use load::{load_binary, read_manifest, read_manifest_raw, read_proto_files, read_signature};
pub use manifest::{BinaryEntry, ManifestMetadata, PackageManifest, ProtoFileEntry, ResourceEntry};
pub use pack::{PackOptions, pack};
pub use verify::{VerifiedPackage, verify};

/// Compute deterministic key_id from Ed25519 public key bytes.
///
/// Algorithm: `"mfr-" + hex(sha256(public_key_bytes))[..16]`
///
/// This MUST match the server-side implementation in `actrix-mfr::crypto::compute_key_id`.
pub fn compute_key_id(public_key_bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(public_key_bytes);
    let hex_str: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("mfr-{}", &hex_str[..16])
}
