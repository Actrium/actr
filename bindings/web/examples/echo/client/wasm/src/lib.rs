//! Echo Client WASM - local handler + SW runtime
//!
//! Demonstrates the local-handler pattern:
//! - register the local `SendEcho` service handler
//! - use `ctx.discover()` to find the remote Echo server
//! - forward the request through `ctx.call_raw()` and return the response
//!
//! ## Architecture
//!
//! ```text
//! DOM (UI)
//!   │  callRaw("echo.SendEcho.SendEcho", payload)
//!   ▼
//! handle_dom_control → SERVICE_HANDLER (dispatcher)
//!   │  handler(route_key, payload, ctx)
//!   │    └─ match "echo.SendEcho" service
//!   │    └─ send_echo_handler::handle_request("SendEcho", payload, ctx)
//!   │       └─ ctx.discover(target_type) → ActrId
//!   │       └─ ctx.call_raw(target, "echo.EchoService.Echo", payload) → response
//!   ▼
//! DOM (UI) ← control_response(response)
//! ```

mod generated;
mod send_echo_handler;

use std::rc::Rc;

use wasm_bindgen::prelude::*;

// Re-export the public SW runtime API.
pub use actr_runtime_sw::*;

/// WASM initialization entry point
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
    wasm_logger::init(wasm_logger::Config::default());

    log::info!("Echo Client WASM initialized");
    log::info!("  - SW Runtime: included");
    log::info!("  - Local Service: echo.SendEcho");
}

/// Register the local `SendEcho` service handler.
///
/// The handler dispatches RPC requests to the concrete `SendEcho` methods:
/// - `echo.SendEcho.SendEcho`: discovers the remote `EchoService` and forwards the request
#[wasm_bindgen]
pub fn register_echo_client_handler() {
    log::info!("Registering local SendEcho handler...");

    actr_runtime_sw::register_service_handler(Rc::new(|route_key, bytes, ctx| {
        let route_key = route_key.to_string();
        let bytes = bytes.to_vec();
        Box::pin(async move {
            // Parse route_key: "echo.SendEcho.SendEcho" → service="echo.SendEcho", method="SendEcho"
            let (service, method) = if let Some(last_dot) = route_key.rfind('.') {
                (&route_key[..last_dot], &route_key[last_dot + 1..])
            } else {
                (route_key.as_str(), "")
            };
            match service {
                "echo.SendEcho" => send_echo_handler::handle_request(method, &bytes, ctx).await,
                _ => Err(format!("Unknown service: {}", service)),
            }
        })
    }));

    log::info!("Local SendEcho handler registered (proxy to remote EchoService)");
}
