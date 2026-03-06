//! Echo Server WASM - 用户 Workload + SW Runtime
//!
//! 此 crate 将用户代码（EchoService）与 SW Runtime 框架代码一起编译成 WASM。
//!
//! ## 架构
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
//! ## 编译产物
//!
//! ```text
//! server/public/
//! ├── echo_server_bg.wasm   # WASM 主体（用户代码 + 框架）
//! ├── echo_server.js        # JS 胶水层
//! └── echo_server.d.ts      # TypeScript 类型
//! ```

mod echo_service;
mod generated;

use std::rc::Rc;

use wasm_bindgen::prelude::*;

// 重新导出 SW Runtime 的公共 API
pub use actr_runtime_sw::*;

pub use echo_service::EchoService;

/// WASM 初始化入口
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
    wasm_logger::init(wasm_logger::Config::default());

    log::info!("Echo Server WASM initialized");
    log::info!("  - SW Runtime: included");
    log::info!("  - User Workload: EchoService");
}

/// 注册 EchoService handler
///
/// handler 分发 RPC 请求到 EchoService 的具体方法，
/// 并将 RuntimeContext 透传给每个方法。
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
                "echo.EchoService" => {
                    echo_service::handle_request(method, &bytes, ctx).await
                }
                _ => Err(format!("Unknown service: {}", service)),
            }
        })
    }));

    log::info!("EchoService registered successfully (handler bound to runtime)");
}
