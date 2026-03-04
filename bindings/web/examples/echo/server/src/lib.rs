//! Echo Server WASM - 用户 Workload + SW Runtime 打包成一个 WASM
//!
//! 此 crate 将用户代码（EchoService）与 SW Runtime 框架代码一起编译成 WASM。
//! 最终产物包含：
//! - SW Runtime（消息路由、Mailbox、传输层等）
//! - 用户 Workload（EchoService 业务逻辑）
//!
//! ## 编译产物
//!
//! ```text
//! public/
//! ├── echo_server.wasm          # WASM 主体（用户代码 + 框架）
//! ├── echo_server.js            # JS 胶水层
//! └── echo_server.d.ts          # TypeScript 类型
//! ```
//!
//! ## RPC 消息处理流程 (State Path)
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │  1. WebRTC DataChannel 接收二进制数据                        │
//! │  2. 数据通过 PostMessage 转发到 Service Worker              │
//! │  3. SW Runtime 解码 RpcEnvelope                             │
//! │  4. Mailbox 入队 → Scheduler 调度                           │
//! │  5. 路由到 EchoService Workload                             │
//! │  6. Workload 处理业务逻辑                                    │
//! │  7. 响应编码为 RpcEnvelope                                   │
//! │  8. 通过 DataChannel 返回给调用方                            │
//! └─────────────────────────────────────────────────────────────┘
//! ```

mod echo_service;
mod generated;

use wasm_bindgen::prelude::*;

// 重新导出 SW Runtime 的公共 API
// 这样 JS 胶水层可以调用 runtime 的初始化函数
pub use actr_runtime_sw::*;

pub use echo_service::EchoService;

/// 初始化 WASM 模块
///
/// 在 wasm-bindgen 加载完成后自动调用
#[wasm_bindgen(start)]
pub fn init() {
    // 设置 panic hook 以便调试
    console_error_panic_hook::set_once();

    // 初始化日志
    wasm_logger::init(wasm_logger::Config::default());

    log::info!("Echo Server WASM initialized");
    log::info!("  - SW Runtime: included");
    log::info!("  - User Workload: EchoService");
}

/// 注册用户 Workload
///
/// 在 runtime 初始化后调用，注册 EchoService 作为 Workload
/// 将 EchoService 的 handle_request 绑定到 SW Runtime 的服务处理器
#[wasm_bindgen]
pub fn register_echo_service() {
    use std::rc::Rc;

    log::info!("Registering EchoService workload...");

    actr_runtime_sw::register_service_handler(Rc::new(|route_key, bytes, _ctx| {
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
                "echo.EchoService" => echo_service::handle_request(method, &bytes).await,
                _ => Err(format!("Unknown service: {}", service)),
            }
        })
    }));

    log::info!("EchoService registered successfully (handler bound to runtime)");
}

/// 处理 RPC 请求（临时 API，供测试用）
///
/// 生产环境中，RPC 请求由 SW Runtime 的 Mailbox + Scheduler 处理
#[wasm_bindgen]
pub async fn handle_rpc(
    service_name: &str,
    method_name: &str,
    request_bytes: &[u8],
) -> Result<Vec<u8>, JsValue> {
    log::debug!(
        "handle_rpc: service={}, method={}, payload_len={}",
        service_name,
        method_name,
        request_bytes.len()
    );

    match service_name {
        "echo.EchoService" => echo_service::handle_request(method_name, request_bytes)
            .await
            .map_err(|e| JsValue::from_str(&e)),
        _ => Err(JsValue::from_str(&format!(
            "Unknown service: {}",
            service_name
        ))),
    }
}

/// 获取服务信息
#[wasm_bindgen]
pub fn get_service_info() -> JsValue {
    let info = serde_json::json!({
        "services": [
            {
                "name": "echo.EchoService",
                "methods": ["echo"]
            }
        ],
        "version": env!("CARGO_PKG_VERSION"),
        "runtime": "actr-runtime-sw",
    });

    serde_wasm_bindgen::to_value(&info).unwrap_or(JsValue::NULL)
}
