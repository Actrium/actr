//! Echo Service 实现
//!
//! 处理 Echo RPC 请求。
//! echo 方法签名包含 RuntimeContext 参数，后续由 proto 代码生成器自动生成。

use std::rc::Rc;

use crate::generated::echo::{EchoRequest, EchoResponse};
use actr_runtime_sw::RuntimeContext;
use prost::Message;

/// Echo Service 实现
pub struct EchoService;

impl EchoService {
    pub fn new() -> Self {
        Self
    }

    /// 处理 Echo 请求
    ///
    /// 方法签名包含 `ctx: Rc<RuntimeContext>`，后续将由 proto 生成。
    /// 当前 echo 场景无需使用 ctx（纯本地处理），但保留参数以匹配生成的签名。
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

/// 全局服务实例
static SERVICE: std::sync::OnceLock<EchoService> = std::sync::OnceLock::new();

fn get_service() -> &'static EchoService {
    SERVICE.get_or_init(EchoService::new)
}

/// 处理 RPC 请求
///
/// 由 register_echo_service 注册的 handler 调用。
/// ctx 透传给具体的 service method。
pub async fn handle_request(
    method: &str,
    request_bytes: &[u8],
    ctx: Rc<RuntimeContext>,
) -> Result<Vec<u8>, String> {
    match method {
        "echo" | "Echo" => {
            // 解码请求
            let request = EchoRequest::decode(request_bytes)
                .map_err(|e| format!("Failed to decode EchoRequest: {}", e))?;

            // 调用服务（传入 ctx）
            let response = get_service().echo(request, ctx).await?;

            // 编码响应
            let mut buf = Vec::with_capacity(response.encoded_len());
            response
                .encode(&mut buf)
                .map_err(|e| format!("Failed to encode EchoResponse: {}", e))?;

            Ok(buf)
        }
        _ => Err(format!("Unknown method: {}", method)),
    }
}
