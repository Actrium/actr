//! Echo Client WASM - 本地 Handler + SW Runtime
//!
//! 演示 local handler 模式（对应 Kotlin 的 UnifiedHandler + ContextBridge）：
//! - 注册 SendEcho 本地服务 handler
//! - handler 通过 `ctx.discover()` 发现远程 Echo Server
//! - handler 通过 `ctx.call_raw()` 将请求转发给远程并获取响应
//!
//! ## 架构
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

// 重新导出 SW Runtime 的公共 API
pub use actr_runtime_sw::*;

/// WASM 初始化入口
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
    wasm_logger::init(wasm_logger::Config::default());

    log::info!("Echo Client WASM initialized");
    log::info!("  - SW Runtime: included");
    log::info!("  - Local Service: echo.SendEcho");
}

/// 注册本地 SendEcho 服务 handler
///
/// handler 分发 RPC 请求到 SendEcho 服务的具体方法：
/// - `echo.SendEcho.SendEcho`: 发现远程 EchoService 并转发请求
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
