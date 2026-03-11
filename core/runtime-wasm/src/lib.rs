//! # actr-runtime-wasm
//!
//! Actor-RTC WASM guest-side runtime, runs on `wasm32-unknown-unknown` target.
//!
//! This crate is the WASM guest-side counterpart of `actr-runtime` (native host-side):
//! - **`actr-runtime`**: native host, drives WASM module execution (Wasmtime / WAMR)
//! - **`actr-runtime-wasm`**: WASM guest-side, compiled into WASM binary, provides `Context` impl
//!
//! ## Architecture position
//!
//! ```text
//! actor business code (actr-framework interface)
//!         | compiled to wasm32
//! actr-runtime-wasm (this crate, compiled into WASM binary)
//!         | host imports
//! actr-runtime (native, host-side WasmHost/WasmInstance)
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use actr_runtime_wasm::entry;
//!
//! // 1. Implement Handler (via actr-framework interface)
//! struct MyService;
//! // impl EchoServiceHandler for MyService { ... }
//!
//! // 2. Register Workload, generate WASM ABI exports
//! entry!(EchoServiceWorkload<MyService>);
//! // Or custom initialization:
//! // entry!(EchoServiceWorkload<MyService>, EchoServiceWorkload(MyService::new()));
//! ```
//!
//! ## asyncify transparent suspend
//!
//! `WasmContext::call(...)` and other communication methods internally call synchronous host imports.
//! After compilation, the WASM binary is transformed by `wasm-opt --asyncify`, enabling host import
//! call sites to be transparently suspended/resumed by the host without modifying business code.

pub mod abi;
pub mod context;
pub mod executor;
pub mod imports;

// Convenience re-exports
pub use context::WasmContext;
