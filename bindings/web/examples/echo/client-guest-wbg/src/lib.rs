// SPDX-License-Identifier: Apache-2.0

//! Echo **client** guest — Option U / Phase 4a variant.
//!
//! Ports the transparent-proxy business logic of `../client-guest/`
//! onto the wasm-bindgen + `actr-web-abi` pipeline. The proxy is
//! deliberately thin: on every inbound `dispatch` the workload
//! discovers `acme:EchoService:0.1.0` and forwards the raw bytes
//! via `host::call_raw`.
//!
//! Build with:
//!
//! ```bash
//! wasm-pack build --target no-modules --release --out-dir pkg
//! ```
//!
//! Phase 4a scope: compile + emit valid wasm/js. Phase 4c will
//! replace the stub `actr-host.js` shim with the real sw-host
//! dispatcher and drive the full client → server round-trip.

use actr_web_abi::guest as host_api;
use actr_web_abi::host::{Workload, set_workload};
use actr_web_abi::types::{
    ActrError, ActrType, BackpressureEvent, CredentialEvent, Dest, ErrorEvent, PeerEvent,
    RpcEnvelope,
};
use async_trait::async_trait;
use wasm_bindgen::prelude::*;

/// Target `ActrType` the client proxy resolves against. Kept in sync
/// with the legacy Component-Model client guest (`../client-guest/`).
const ECHO_SERVICE_MANUFACTURER: &str = "acme";
const ECHO_SERVICE_NAME: &str = "EchoService";
const ECHO_SERVICE_VERSION: &str = "0.1.0";

/// Client workload: discovers an echo server on every `dispatch` and
/// forwards the payload untouched. The legacy crate keeps a cache;
/// for the Phase 4a smoke test we drop the cache to keep the surface
/// minimal — correctness/perf tuning lives in Phase 4c.
pub struct EchoClientWorkload;

#[async_trait(?Send)]
impl Workload for EchoClientWorkload {
    async fn dispatch(&self, envelope: RpcEnvelope) -> Result<Vec<u8>, ActrError> {
        let target_type = ActrType {
            manufacturer: ECHO_SERVICE_MANUFACTURER.to_string(),
            name: ECHO_SERVICE_NAME.to_string(),
            version: ECHO_SERVICE_VERSION.to_string(),
        };

        // Discover a reachable server. `discover` returns
        // `Result<Result<ActrId, ActrError>, JsValue>`: the outer
        // layer is the JS transport error (bridge missing /
        // Promise rejected), the inner layer is the WIT-declared
        // host error (no candidate registered, etc.).
        let server_id = host_api::discover(target_type)
            .await
            .map_err(|e| ActrError::Internal(format!("host discover transport error: {e:?}")))?
            .map_err(|e| ActrError::Internal(format!("host discover: {e:?}")))?;

        // Forward the raw payload via `call_raw`, keeping the same
        // route-key the inbound envelope carried.
        host_api::call_raw(server_id, envelope.route_key, envelope.payload)
            .await
            .map_err(|e| ActrError::Internal(format!("host call_raw transport error: {e:?}")))?
    }

    async fn on_start(&self) -> Result<(), ActrError> {
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
    // Empty bodies rather than `todo!()` so the module links cleanly
    // even if the host happens to call these before real wiring is in
    // place. Real per-transition logic (invalidate cache on
    // webrtc-disconnected, etc.) belongs to Phase 4c.

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

/// Unused helper that keeps `Dest` referenced from user code so IDE
/// jump-to-def and wasm-opt dead-code elimination both treat it as
/// "live". Real routing would pass `Dest::Actor(server_id)` to `tell`
/// or `call`; the Phase 4a smoke test uses `call_raw` which already
/// takes `ActrId` directly.
#[allow(dead_code)]
fn _dest_touched() -> Dest {
    Dest::Local
}

/// Bootstrap hook — wasm-bindgen calls this on module instantiation,
/// installing the single workload singleton before any export runs.
#[wasm_bindgen(start)]
pub fn __actr_guest_bootstrap() -> Result<(), JsValue> {
    set_workload(EchoClientWorkload).map_err(JsValue::from_str)
}
