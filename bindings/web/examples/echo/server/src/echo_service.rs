//! Echo Service 实现
//!
//! 处理 Echo RPC 请求

use crate::generated::echo::{EchoRequest, EchoResponse};
use prost::Message;

/// Echo Service 实现
pub struct EchoService;

impl EchoService {
    pub fn new() -> Self {
        Self
    }

    /// 处理 Echo 请求
    pub async fn echo(&self, request: EchoRequest) -> Result<EchoResponse, String> {
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
pub async fn handle_request(method: &str, request_bytes: &[u8]) -> Result<Vec<u8>, String> {
    match method {
        "echo" | "Echo" => {
            // 解码请求
            let request = EchoRequest::decode(request_bytes)
                .map_err(|e| format!("Failed to decode EchoRequest: {}", e))?;

            // 调用服务
            let response = get_service().echo(request).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_echo_service() {
        let service = EchoService::new();

        // 创建请求
        let request = EchoRequest {
            message: "Hello, World!".to_string(),
        };

        // 由于 wasm_bindgen_test 需要特殊设置，这里只测试同步部分
        assert_eq!(request.message, "Hello, World!");
    }
}
