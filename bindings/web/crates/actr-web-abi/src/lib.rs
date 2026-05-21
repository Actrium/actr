// SPDX-License-Identifier: Apache-2.0

//! ABI crate generated from `core/framework/wit/actr-workload.wit` via
//! `tools/wit-compile-web`. Do not hand-edit `src/{types,guest,host}.rs`
//! — regenerate with `cargo run -p actr-wit-compile-web` instead.
//!
//! This is the browser-path analogue of the host-side wit-bindgen
//! output: it exposes the WIT contract's record / variant types, the
//! host-imported functions the guest calls into, and the workload
//! entry points the host dispatches into. No Component Model runtime
//! is involved; everything rides wasm-bindgen + serde-wasm-bindgen.

pub mod guest;
pub mod host;
pub mod types;
