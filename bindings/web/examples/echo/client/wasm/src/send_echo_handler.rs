//! SendEcho 本地服务实现
//!
//! 处理 SendEcho RPC 请求：发现远程 EchoService 并转发请求。
//!
//! 对应 proto 定义：
//! ```proto
//! service SendEcho {
//!   rpc SendEcho(EchoRequest) returns (EchoResponse);
//! }
//! ```

use std::rc::Rc;
use std::sync::OnceLock;

use actr_runtime_sw::RuntimeContext;
use actr_runtime_sw::WebContext;

/// 远程 Echo Server 的 ActrType
static ECHO_SERVER_TYPE: OnceLock<actr_runtime_sw::actr_protocol::ActrType> = OnceLock::new();

fn echo_server_type() -> &'static actr_runtime_sw::actr_protocol::ActrType {
    ECHO_SERVER_TYPE.get_or_init(|| actr_runtime_sw::actr_protocol::ActrType {
        manufacturer: "acme".to_string(),
        name: "EchoService".to_string(),
        version: "v1".to_string(),
    })
}

/// 处理 SendEcho 服务的 RPC 请求
///
/// 由 register_echo_client_handler 注册的 handler 调用。
/// 当前仅支持 `SendEcho` 方法：发现远程 EchoService 并转发请求。
pub async fn handle_request(
    method: &str,
    request_bytes: &[u8],
    ctx: Rc<RuntimeContext>,
) -> Result<Vec<u8>, String> {
    match method {
        "SendEcho" => {
            log::info!("[SendEcho] Received request, discovering remote EchoService...");

            // 1. 发现远程 Echo Server
            let target = ctx
                .discover(echo_server_type())
                .await
                .map_err(|e| format!("Discover failed: {}", e))?;

            log::info!("[SendEcho] Discovered echo server: {:?}", target);

            // 2. 通过 ctx.call_raw() 转发请求到远程 echo.EchoService.Echo
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
