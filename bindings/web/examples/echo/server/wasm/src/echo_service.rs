//! `EchoService` implementation.
//!
//! Handles Echo RPC requests.
//! The eventual generated signature includes `RuntimeContext`.

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
    ///
    /// The generated signature includes `ctx: Rc<RuntimeContext>`.
    /// The Echo flow does not need it today because processing is local, but
    /// the parameter is kept to match the generated shape.
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
/// The context is forwarded into the concrete service method.
pub async fn handle_request(
    method: &str,
    request_bytes: &[u8],
    ctx: Rc<RuntimeContext>,
) -> Result<Vec<u8>, String> {
    match method {
        "echo" | "Echo" => {
            // Decode the request.
            let request = EchoRequest::decode(request_bytes)
                .map_err(|e| format!("Failed to decode EchoRequest: {}", e))?;

            // Invoke the service with the forwarded context.
            let response = get_service().echo(request, ctx).await?;

            // Encode the response.
            let mut buf = Vec::with_capacity(response.encoded_len());
            response
                .encode(&mut buf)
                .map_err(|e| format!("Failed to encode EchoResponse: {}", e))?;

            Ok(buf)
        }
        _ => Err(format!("Unknown method: {}", method)),
    }
}
