//! WASM workload execution engine (feature = "wasm-engine").

pub mod abi;
pub mod component_bindings;
pub mod error;
pub mod host;

pub use error::{WasmError, WasmResult};
pub use host::{WasmHost, WasmWorkload};
