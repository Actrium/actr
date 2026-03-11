//! Dynclib (cdylib) guest-side runtime module
//!
//! Runs in native shared libraries (.so/.dylib/.dll). Provides `DynclibContext`
//! (Context impl) that communicates with the host via `HostVTable` function pointers.

pub mod context;

pub use context::DynclibContext;
