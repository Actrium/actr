//! Component Model bindings generated from `core/framework/wit/actr-workload.wit`.
//!
//! Re-runs `wit_bindgen::generate!` once per compiled guest; the emitted
//! types live under `exports::actr::workload::workload` (the guest-provided
//! `Guest` trait) and `actr::workload::{host, types}` (host imports consumed
//! by the guest).
//!
//! Only compiled for `wasm32-wasip2` — the surrounding `guest::wasm` module
//! is `#[cfg(target_arch = "wasm32")]`-gated, so hosts never see this code
//! or the underlying `wit-bindgen` crate.
//!
//! # Async flag
//!
//! `async: true` is load-bearing: wasmtime's Component Model async binding
//! on the host side expects async-ABI custom sections on the guest, which
//! wit-bindgen only emits under this flag. With the flag set, every host
//! import appears as an `async fn` at the Rust surface (including the
//! otherwise-sync `log_message`) — the tradeoff accepted by the Phase 0.5
//! spike.
//!
//! # `generate_all`
//!
//! The WIT world imports `host` and exports `workload`; `generate_all`
//! tells wit-bindgen to emit bindings for both sides. Without it only the
//! exports surface is generated.

wit_bindgen::generate!({
    world: "actr-workload-guest",
    path: "wit",
    async: true,
    generate_all,
    // The `entry!` macro in `guest/mod.rs` expands inside the user's crate
    // and needs to call `export!` from user-crate scope. Default `export!`
    // is crate-private; `pub_export_macro: true` makes it `pub` so it can
    // be invoked via `::actr_framework::guest::wasm::generated::export!(...)`.
    pub_export_macro: true,
});
