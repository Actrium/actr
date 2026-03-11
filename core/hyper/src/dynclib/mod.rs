//! Dynclib actor execution engine
//!
//! Loads native shared libraries (.so/.dylib/.dll) compiled as cdylib actors.
//! Provides [`DynclibHost`] (library loader) and [`DynclibInstance`] (per-actor runtime)
//! implementing [`ExecutorAdapter`](crate::executor::ExecutorAdapter).

mod error;
mod host;

pub use error::{DynclibError, DynclibResult};
pub use host::{DynclibHost, DynclibInstance};
