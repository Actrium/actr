//! WASM actor execution engine (feature = "wasm-engine")
//!
//! Provides [`WasmHost`] and [`WasmInstance`], alongside the native actor's tokio execution engine.

pub mod abi;
pub mod error;
pub mod host;

pub use abi::WasmActorConfig;
pub use error::{WasmError, WasmResult};
pub use host::{WasmHost, WasmInstance};

// Re-export shared executor types for backward compatibility
pub use crate::executor::{DispatchContext, IoResult, PendingCall};
