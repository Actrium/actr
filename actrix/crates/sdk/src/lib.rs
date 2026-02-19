//! Unified SDK facade for Actrix.
//!
//! This crate organizes exports into explicit layers:
//! - `control`: stable control-plane API facade.
//! - `testing`: internal integration-test oriented facade (feature-gated).
//! - `compat`: historical naming aliases.

pub mod compat;
pub mod control;
#[cfg(feature = "testing")]
pub mod testing;

// Backward-compatible top-level exports remain available.
pub use control::*;

// Compatibility aliases are intentionally re-exported at top-level so
// historical imports continue to compile during migration.
pub use compat::{Result, Supervisord, SupervitClient, SupervitConfig, SupervitError};
