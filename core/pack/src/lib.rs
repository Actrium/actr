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
pub use load::{load_binary, read_manifest};
pub use manifest::{BinaryEntry, ManifestMetadata, PackageManifest, ResourceEntry};
pub use pack::{pack, PackOptions};
pub use verify::verify;
