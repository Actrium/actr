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
//!
//! Phase 6b adds [`adapter::WebWorkloadAdapter`], the adapter the
//! `actr_framework::entry!` macro's `feature = "web"` branch wraps around
//! the user's `Workload` so it can be handed to
//! `actr_web_abi::host::register_workload`.

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub mod adapter;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub mod context;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use adapter::WebWorkloadAdapter;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use context::WebContext;

/// Stable re-export surface for the `entry!` macro's `feature = "web"`
/// branch. The macro expands inside the user crate, so every item it
/// references must be reachable from an external crate path — this
/// module is that single stable prefix.
///
/// Items here are `#[doc(hidden)]`: they exist only to wire the macro
/// expansion together. Downstream crates must not depend on them
/// directly; the public surface is the `entry!` macro plus the
/// cross-target `Workload` / `Context` traits.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[doc(hidden)]
pub mod __web_macro_support {
    pub use super::adapter::WebWorkloadAdapter;
    // The macro registers the adapter through the web ABI crate's
    // `register_workload` free function and tags the bootstrap fn with
    // `wasm_bindgen(start)` so the browser runtime invokes it on
    // module instantiation.
    pub use actr_web_abi::host::register_workload;
    pub use wasm_bindgen::prelude::wasm_bindgen;
}
