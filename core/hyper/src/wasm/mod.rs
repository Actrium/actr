//! WASM workload execution engine (feature = "wasm-engine").
//!
//! Backed by the Component Model (wasmtime 43 + wit-bindgen) as of
//! Phase 1 Commit 2; see `core/framework/wit/actr-workload.wit` for the
//! contract.

pub(crate) mod component_bindings;
mod error;
mod host;

pub use error::WasmError;
pub use host::WasmHost;
pub(crate) use host::WasmWorkload;
