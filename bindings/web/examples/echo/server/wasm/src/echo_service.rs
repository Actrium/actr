//! `EchoService` implementation for Web.
//!
//! Handles Echo RPC requests in the browser Service Worker environment.

use std::rc::Rc;

use crate::generated::echo::{EchoRequest, EchoResponse};
use actr_runtime_sw::RuntimeContext;
use prost::Message;

/// `EchoService` implementation.
pub struct EchoService;

impl EchoService {
    pub fn new() -> Self {
        Self
    }

    /// Handle an Echo request.
    pub async fn echo(
        &self,
        request: EchoRequest,
        _ctx: Rc<RuntimeContext>,
    ) -> Result<EchoResponse, String> {
        log::info!("📨 Received Echo request: message='{}'", request.message);

        let reply = format!("Echo: {}", request.message);
        let timestamp = js_sys::Date::now() as u64 / 1000;

        log::info!("📤 Sending Echo response: reply='{}'", reply);

        Ok(EchoResponse { reply, timestamp })
    }
}

impl Default for EchoService {
    fn default() -> Self {
        Self::new()
    }
}

/// Global service instance.
static SERVICE: std::sync::OnceLock<EchoService> = std::sync::OnceLock::new();

fn get_service() -> &'static EchoService {
    SERVICE.get_or_init(EchoService::new)
}

/// Handle an RPC request.
///
/// Called by the handler registered through `register_echo_service`.
pub async fn handle_request(
    method: &str,
    request_bytes: &[u8],
    ctx: Rc<RuntimeContext>,
) -> Result<Vec<u8>, String> {
    match method {
        "echo" | "Echo" => {
            let request = EchoRequest::decode(request_bytes)
                .map_err(|e| format!("Failed to decode EchoRequest: {}", e))?;

            let response = get_service().echo(request, ctx).await?;

            let mut buf = Vec::with_capacity(response.encoded_len());
            response
                .encode(&mut buf)
                .map_err(|e| format!("Failed to encode EchoResponse: {}", e))?;

            Ok(buf)
        }
        _ => Err(format!("Unknown method: {}", method)),
    }
}
