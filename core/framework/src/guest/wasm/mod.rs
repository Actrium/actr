//! WASM guest-side runtime module
//!
//! Runs on `wasm32-unknown-unknown` target. Provides `WasmContext` (Context impl)
//! and host import declarations for transparent asyncify suspend/resume.

pub mod context;
pub mod executor;
pub mod imports;

pub use context::WasmContext;
