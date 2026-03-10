//! WASM actor 执行引擎（feature = "wasm-engine"）
//!
//! 提供 [`WasmHost`] 和 [`WasmInstance`]，与 native actor 的 tokio 执行引擎并列。

pub mod abi;
pub mod error;
pub mod host;

pub use abi::WasmActorConfig;
pub use error::{WasmError, WasmResult};
pub use host::{DispatchContext, IoResult, PendingCall, WasmHost, WasmInstance};
