//! Web-target runtime glue for `actr-framework`.
//!
//! Compiled only when both conditions hold:
//! - target is `wasm32` (so wasm-bindgen and friends are usable);
//! - the `web` cargo feature is enabled (so the extra deps are actually
//!   pulled in).
//!
//! Native builds and `wasm32-wasip2` Component Model builds never see this
//! module.
//!
//! Per Option U γ-unified §3.3 the `WebContext` struct implements the
//! shared `Context` trait for the wasm-bindgen (`wasm32-unknown-unknown`)
//! path, capturing `(self_id, caller_id, request_id)` once at dispatch
//! construction and exposing it through the same trait the `wasip2`
//! `WasmContext` / native `RuntimeContext` implement. Users consume it
//! via the `Context` trait only; they never name `WebContext` directly.

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub mod context;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use context::WebContext;
