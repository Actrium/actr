// SPDX-License-Identifier: Apache-2.0

//! Compile-only smoke test for Option U Phase 6b.
//!
//! Exercises the `actr_framework::entry!` macro's `feature = "web"`
//! branch end-to-end:
//!
//! - Construct a minimal `Workload` + `MessageDispatcher` pair so the
//!   macro has something concrete to wire.
//! - Invoke `entry!(SmokeWorkload)` — this expands to the new
//!   `#[wasm_bindgen(start)]` bootstrap that wraps the workload in
//!   `actr_framework::web::WebWorkloadAdapter` and calls
//!   `actr_web_abi::host::register_workload`.
//!
//! Build with:
//!
//! ```bash
//! cargo check -p framework-web-entry-smoke --target wasm32-unknown-unknown
//! ```
//!
//! Success == the macro expansion type-checks against the
//! `actr_web_abi::host::Workload` trait's 17-method contract and the
//! adapter's associated-type / lifetime bounds line up. Failure ==
//! regression in either `entry!` (macro body) or `WebWorkloadAdapter`
//! (trait implementation).

use actr_framework::{Context, MessageDispatcher, Workload, entry};
use actr_protocol::{ActorResult, ActrError, RpcEnvelope};
use async_trait::async_trait;
use bytes::Bytes;

/// Stateless workload. `Clone` satisfies
/// `actr_web_abi::host::register_workload`'s `W: Clone` bound — which
/// `WebWorkloadAdapter` inherits from its inner `W`.
#[derive(Clone, Default)]
pub struct SmokeWorkload;

/// Minimal dispatcher: echoes the envelope payload. Not a functional
/// router — its only job is to witness the
/// `Workload::Dispatcher`/`MessageDispatcher::Workload` type-equality
/// so the macro has something concrete to wire.
pub struct SmokeDispatcher;

#[async_trait(?Send)]
impl MessageDispatcher for SmokeDispatcher {
    type Workload = SmokeWorkload;

    async fn dispatch<C: Context>(
        _workload: &Self::Workload,
        envelope: RpcEnvelope,
        _ctx: &C,
    ) -> ActorResult<Bytes> {
        match envelope.payload {
            Some(b) => Ok(b),
            None => Err(ActrError::DecodeFailure(
                "smoke workload: empty payload".to_string(),
            )),
        }
    }
}

impl Workload for SmokeWorkload {
    type Dispatcher = SmokeDispatcher;
}

// Expands to the Phase 6b web bootstrap. On
// `wasm32-unknown-unknown` + `feature = "web"` this emits:
//
// ```rust,ignore
// const _: () = {
//     use actr_framework::web::__web_macro_support as __m;
//     #[__m::wasm_bindgen(start)]
//     fn __actr_web_bootstrap() {
//         let workload: SmokeWorkload = Default::default();
//         let adapter = __m::WebWorkloadAdapter::new(workload);
//         __m::register_workload(adapter);
//     }
// };
// ```
entry!(SmokeWorkload);
