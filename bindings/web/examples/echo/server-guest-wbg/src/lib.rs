// SPDX-License-Identifier: Apache-2.0

//! Echo **server** guest — Option U / Phase 4a variant.
//!
//! Ports the business logic of `../server-guest/` off the
//! Component-Model + wasm-component-ld pipeline onto the new
//! wasm-bindgen + `actr-web-abi` pipeline. Build with:
//!
//! ```bash
//! wasm-pack build --target no-modules --release --out-dir pkg
//! ```
//!
//! The generated `pkg/*.wasm` is a plain core module that imports
//! `actrHost*` functions from a sibling JS shim (`actr-host.js`) and
//! exports the 17 `workload.*` entry points defined in
//! `actr-workload.wit`.
//!
//! Phase 4a scope: compile + emit valid wasm/js. Runtime wiring to
//! `sw-host` is Phase 4c.

use actr_web_abi::host::{Workload, register_workload};
use actr_web_abi::types::{
    ActrError, BackpressureEvent, CredentialEvent, ErrorEvent, PeerEvent, RpcEnvelope,
};
use async_trait::async_trait;
use wasm_bindgen::prelude::*;

/// Echo server workload. Stateless: every `dispatch` just returns the
/// inbound payload bytes verbatim, which is the simplest function that
/// still exercises the full request/response path.
///
/// `Clone` is required because [`register_workload`] consumes the
/// workload by value and internally leaks a `&'static dyn Workload` —
/// the bound documents the P6-I intent of being able to hand a cloned
/// handle to each per-dispatch `WebContext` once the unified path lands.
#[derive(Clone)]
pub struct EchoServerWorkload;

#[async_trait(?Send)]
impl Workload for EchoServerWorkload {
    /// Echo handler — return the payload as-is. Ignores `route_key`
    /// and `request_id`; the real routing is handled by the framework
    /// on the native side, and for Phase 4a smoke tests we only need
    /// to prove the ABI marshaling compiles.
    async fn dispatch(&self, envelope: RpcEnvelope) -> Result<Vec<u8>, ActrError> {
        Ok(envelope.payload)
    }

    async fn on_start(&self) -> Result<(), ActrError> {
        // Real impl would subscribe to signaling / prime caches. The
        // smoke test only needs a non-trapping default.
        Ok(())
    }

    async fn on_ready(&self) -> Result<(), ActrError> {
        Ok(())
    }

    async fn on_stop(&self) -> Result<(), ActrError> {
        Ok(())
    }

    async fn on_error(&self, _event: ErrorEvent) -> Result<(), ActrError> {
        Ok(())
    }

    // ── Observation hooks (infallible) ───────────────────────────────
    // All intentionally empty: the echo workload has no lifecycle
    // state to update on connection transitions. Keeping them as
    // explicit empty bodies rather than `todo!()` so the wasm
    // module links without panics if the host happens to call them.

    async fn on_credential_expiring(&self, _event: CredentialEvent) {}
    async fn on_credential_renewed(&self, _event: CredentialEvent) {}
    async fn on_mailbox_backpressure(&self, _event: BackpressureEvent) {}
    async fn on_signaling_connected(&self) {}
    async fn on_signaling_connecting(&self) {}
    async fn on_signaling_disconnected(&self) {}
    async fn on_webrtc_connected(&self, _event: PeerEvent) {}
    async fn on_webrtc_connecting(&self, _event: PeerEvent) {}
    async fn on_webrtc_disconnected(&self, _event: PeerEvent) {}
    async fn on_websocket_connected(&self, _event: PeerEvent) {}
    async fn on_websocket_connecting(&self, _event: PeerEvent) {}
    async fn on_websocket_disconnected(&self, _event: PeerEvent) {}
}

/// Bootstrap hook — wasm-bindgen invokes this automatically when the
/// module is instantiated, installing the single workload singleton
/// before any export is called.
///
/// `register_workload` is single-shot: a second call panics. In the
/// browser path each guest module is instantiated exactly once per
/// service-worker session, so this is fine.
#[wasm_bindgen(start)]
pub fn __actr_guest_bootstrap() {
    register_workload(EchoServerWorkload);
}
