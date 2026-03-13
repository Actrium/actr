//! Local `SendEcho` service implementation.
//!
//! Handles `SendEcho` RPC requests by discovering the remote `EchoService`
//! and forwarding the request.
//!
//! Corresponding proto definition:
//! ```proto
//! service SendEcho {
//!   rpc SendEcho(EchoRequest) returns (EchoResponse);
//! }
//! ```

use std::rc::Rc;
use std::sync::OnceLock;

use actr_runtime_sw::RuntimeContext;
use actr_runtime_sw::WebContext;

/// `ActrType` for the remote Echo server.
static ECHO_SERVER_TYPE: OnceLock<actr_runtime_sw::actr_protocol::ActrType> = OnceLock::new();

fn echo_server_type() -> &'static actr_runtime_sw::actr_protocol::ActrType {
    ECHO_SERVER_TYPE.get_or_init(|| actr_runtime_sw::actr_protocol::ActrType {
        manufacturer: "acme".to_string(),
        name: "EchoService".to_string(),
        version: "0.1.0".to_string(),
    })
}

/// Handle RPC requests for the `SendEcho` service.
///
/// Called by the handler registered through `register_echo_client_handler`.
/// Currently only supports the `SendEcho` method: discover the remote
/// `EchoService` and forward the request.
pub async fn handle_request(
    method: &str,
    request_bytes: &[u8],
    ctx: Rc<RuntimeContext>,
) -> Result<Vec<u8>, String> {
    match method {
        "SendEcho" => {
            log::info!("[SendEcho] Received request, discovering remote EchoService...");

            // 1. Discover the remote Echo server.
            let target = ctx
                .discover(echo_server_type())
                .await
                .map_err(|e| format!("Discover failed: {}", e))?;

            log::info!("[SendEcho] Discovered echo server: {:?}", target);

            // 2. Forward the request to remote `echo.EchoService.Echo` via `ctx.call_raw()`.
            let response = ctx
                .call_raw(&target, "echo.EchoService.Echo", request_bytes, 30000)
                .await
                .map_err(|e| format!("call_raw failed: {}", e))?;

            log::info!("[SendEcho] Got response: {} bytes", response.len());

            Ok(response)
        }
        _ => Err(format!("Unknown method in SendEcho service: {}", method)),
    }
}
