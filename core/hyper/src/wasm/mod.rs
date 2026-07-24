//! WASM workload execution engine (feature = "wasm-engine").
//!
//! Backed by the wasmtime 46 Component Model + wit-bindgen against the
//! `actr:workload@0.2.0` async world; see `core/framework/wit-v2/actr-workload.wit`
//! for the contract. The 0.1.0 synchronous world is retired.

pub(crate) mod component_bindings_v2;
mod error;
mod host;
mod host_v2;
mod runtime_limits;

pub use error::WasmError;
pub use host::WasmHost;
pub(crate) use host_v2::WasmWorkloadV2;
pub use runtime_limits::{WasmRuntimeStats, wasm_runtime_stats};
