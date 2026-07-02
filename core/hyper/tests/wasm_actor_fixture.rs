//! Compiled `wasm_actor_fixture` Component Model bytes.
//!
//! The `.wasm` is built by `core/hyper/build.rs` when the `wasm-engine`
//! feature is enabled and the wasm32-wasip2 Component Model toolchain
//! (`wasm-component-ld` >= 0.5.22, `wasm32-wasip2` rustup target) is
//! available. `build.rs` sets the `ACTR_WASM_FIXTURE` env var to the built
//! artifact path and emits the `actr_wasm_fixture_available` cfg.
//!
//! When the toolchain is absent (a developer without the wasm targets
//! installed), `build.rs` emits a `cargo:warning` and omits the cfg, so this
//! constant is absent and the consuming tests are compiled out — no
//! committed binary blob is required.

#[cfg(actr_wasm_fixture_available)]
pub const WASM_ACTOR_FIXTURE: &[u8] = include_bytes!(env!("ACTR_WASM_FIXTURE"));
