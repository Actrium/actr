//! WASM workload execution engine (feature = "wasm-engine").
//!
//! Backed by the Component Model (wasmtime 43 + wit-bindgen) as of
//! Phase 1 Commit 2; see `core/framework/wit/actr-workload.wit` for the
//! contract.

pub mod component_bindings;
pub mod error;
pub mod host;

pub use error::{WasmError, WasmResult};
pub use host::{WasmHost, WasmWorkload};
