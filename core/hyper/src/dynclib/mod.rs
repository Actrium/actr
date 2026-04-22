//! Dynclib actor execution engine
//!
//! Loads native shared libraries (.so/.dylib/.dll) compiled as cdylib actors.
//! Provides [`DynclibHost`] (library loader) and [`DynClibWorkload`] (per-actor runtime).

mod error;
mod host;

pub use error::DynclibError;
pub use host::DynclibHost;
pub(crate) use host::DynClibWorkload;
