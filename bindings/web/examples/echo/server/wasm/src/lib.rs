//! Echo Server WASM for Web - Runtime Host.
//!
//! This crate compiles the SW runtime framework into a WASM artifact for
//! browser deployment. The actual guest business logic (EchoService) is loaded
//! separately as a standard guest WASM at runtime via the guest bridge.

use wasm_bindgen::prelude::*;

// Re-export the public SW runtime API (including guest_bridge).
pub use actr_runtime_sw::*;

/// WASM initialization entry point
#[wasm_bindgen(start)]
pub fn init() {
    // Note: panic hook and logger are initialized by init_global() in the runtime.
    // We only log a marker here to confirm the WASM module loaded.
}
