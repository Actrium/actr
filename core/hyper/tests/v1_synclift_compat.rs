//! M4 compatibility guard: a frozen `actr:workload@0.1.0` sync-lift package
//! must keep loading on the wasmtime 46 host unchanged.
//!
//! Background: M4 introduces the `actr:workload@0.2.0` async world alongside
//! the existing 0.1.0 world (dual-world host). The load path must keep
//! accepting the shipped 0.1.0 sync-lift packages and route them to the
//! serial (V1) execution path — old packages take zero breakage.
//!
//! `fixtures/v1_synclift_guest.wasm` is a frozen artifact of the current
//! `wasm_actor_fixture` guest built by the 0.1.0 (sync-lift, no `async:
//! true`) framework SDK, captured BEFORE the guest SDK was switched to the
//! 0.2.0 async world. Unlike `legacy_asynclift_guest.wasm` (a 43-era
//! async-lift binary that wasmtime 46 rejects), this one is a valid,
//! loadable Component; it is the positive control for "46 host still loads
//! old 0.1.0 sync-lift packages".

#![cfg(feature = "wasm-engine")]

use actr_hyper::wasm::WasmHost;

/// A genuine 0.1.0 sync-lift Component built by the current SDK.
const V1_SYNCLIFT_GUEST: &[u8] = include_bytes!("fixtures/v1_synclift_guest.wasm");

#[test]
fn v1_synclift_package_loads_on_wasmtime_46_host() {
    // The 0.1.0 sync-lift package must compile as a Component without the
    // async-lift rejection that `legacy_asynclift_guest.wasm` triggers.
    WasmHost::compile(V1_SYNCLIFT_GUEST)
        .expect("frozen actr:workload@0.1.0 sync-lift package must load on the wasmtime 46 host");
}
