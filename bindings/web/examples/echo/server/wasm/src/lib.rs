//! Echo Server WASM for Web - user workload + SW runtime.
//!
//! This crate compiles the user code (`EchoService`) together with the SW
//! runtime framework into a single WASM artifact for browser deployment.
//!
//! ## Architecture
//!
//! ```text
//! WebRTC DataChannel
//!   │  RpcEnvelope (binary)
//!   ▼
//! SW Runtime → Mailbox → Scheduler → SERVICE_HANDLER (dispatcher)
//!   │  handler(route_key, payload, ctx)
//!   │    └─ EchoService.echo(request, ctx) → response
//!   ▼
//! WebRTC DataChannel ← RpcEnvelope (response)
//! ```
//!
//! ## Build outputs (via wasm-pack)
//!
//! ```text
//! echo_server_bg.wasm   # Main WASM payload (user code + SW runtime)
//! echo_server.js        # JS glue
//! echo_server.d.ts      # TypeScript type definitions
//! ```

mod echo_service;

pub mod generated {
    pub mod echo {
        include!(concat!(env!("OUT_DIR"), "/echo.rs"));
    }
}

use std::rc::Rc;

use wasm_bindgen::prelude::*;

// Re-export the public SW runtime API.
pub use actr_runtime_sw::*;

pub use echo_service::EchoService;

/// WASM initialization entry point
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
    wasm_logger::init(wasm_logger::Config::default());

    log::info!("Echo Server WASM initialized");
    log::info!("  - SW Runtime: included");
    log::info!("  - User Workload: EchoService");
}

/// Register the `EchoService` handler.
///
/// The handler dispatches RPC requests to the concrete `EchoService` methods
/// and forwards `RuntimeContext` into each method.
#[wasm_bindgen]
pub fn register_echo_service() {
    log::info!("Registering EchoService workload...");

    actr_runtime_sw::register_service_handler(Rc::new(|route_key, bytes, ctx| {
        let route_key = route_key.to_string();
        let bytes = bytes.to_vec();
        Box::pin(async move {
            // Parse route_key: "echo.EchoService.Echo" → service="echo.EchoService", method="Echo"
            let (service, method) = if let Some(last_dot) = route_key.rfind('.') {
                (&route_key[..last_dot], &route_key[last_dot + 1..])
            } else {
                (route_key.as_str(), "")
            };
            match service {
                "echo.EchoService" => echo_service::handle_request(method, &bytes, ctx).await,
                _ => Err(format!("Unknown service: {}", service)),
            }
        })
    }));

    log::info!("EchoService registered successfully (handler bound to runtime)");
}
