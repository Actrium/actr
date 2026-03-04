//! Echo Client WASM - 本地 Handler + SW Runtime
//!
//! 演示 local handler 模式（对应 Kotlin 的 UnifiedHandler + ContextBridge）：
//! - 注册 UnifiedDispatcher 风格的 handler
//! - handler 通过 `ctx.discover()` 发现远程 Echo Server
//! - handler 通过 `ctx.call_raw()` 将请求转发给远程并获取响应
//!
//! ## 架构
//!
//! ```text
//! DOM (UI)
//!   │  callRaw("echo.EchoService.Echo", payload)
//!   ▼
//! handle_dom_control → SERVICE_HANDLER (UnifiedDispatcher)
//!   │  handler(route_key, payload, ctx)
//!   │    └─ ctx.discover(target_type) → ActrId
//!   │    └─ ctx.call_raw(target, route_key, payload) → response
//!   ▼
//! DOM (UI) ← control_response(response)
//! ```

mod generated;

use std::rc::Rc;
use std::sync::OnceLock;

use wasm_bindgen::prelude::*;

// 重新导出 SW Runtime 的公共 API
pub use actr_runtime_sw::*;

// RuntimeContext 已通过上面的 glob re-export 导入，无需重复引用

/// 远程 Echo Server 的 ActrType
static ECHO_SERVER_TYPE: OnceLock<actr_runtime_sw::actr_protocol::ActrType> = OnceLock::new();

fn echo_server_type() -> &'static actr_runtime_sw::actr_protocol::ActrType {
    ECHO_SERVER_TYPE.get_or_init(|| actr_runtime_sw::actr_protocol::ActrType {
        manufacturer: "acme".to_string(),
        name: "EchoService".to_string(),
    })
}

/// WASM 初始化入口
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
    wasm_logger::init(wasm_logger::Config::default());

    log::info!("Echo Client WASM initialized");
    log::info!("  - SW Runtime: included");
    log::info!("  - Local Handler: EchoClientHandler (proxy to remote)");
}

/// 注册本地 handler（UnifiedDispatcher 模式）
///
/// handler 接收所有来自 DOM 的 RPC 请求，然后：
/// 1. 通过 ctx.discover() 发现远程 Echo Server
/// 2. 通过 ctx.call_raw() 转发请求到远程
/// 3. 返回远程响应给 DOM
#[wasm_bindgen]
pub fn register_echo_client_handler() {
    log::info!("Registering EchoClient local handler...");

    actr_runtime_sw::register_service_handler(Rc::new(
        |route_key: &str, bytes: &[u8], ctx: Rc<RuntimeContext>| {
            let route_key = route_key.to_string();
            let bytes = bytes.to_vec();
            Box::pin(async move {
                log::info!(
                    "[EchoClientHandler] Received request: route_key={}",
                    route_key
                );

                // 1. 发现远程 Echo Server
                let target = ctx
                    .discover(echo_server_type())
                    .await
                    .map_err(|e| format!("Discover failed: {}", e))?;

                log::info!("[EchoClientHandler] Discovered echo server: {:?}", target);

                // 2. 通过 ctx.call_raw() 转发请求到远程 Echo Server
                let response = ctx
                    .call_raw(&target, &route_key, &bytes, 30000)
                    .await
                    .map_err(|e| format!("call_raw failed: {}", e))?;

                log::info!("[EchoClientHandler] Got response: {} bytes", response.len());

                Ok(response)
            })
        },
    ));

    log::info!("EchoClient local handler registered (proxy to remote echo server)");
}
