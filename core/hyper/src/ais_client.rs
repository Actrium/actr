//! AIS HTTP 客户端
//!
//! 封装向 AIS `/register` 端点发送 protobuf 请求的逻辑。
//! 支持两种注册模式：
//! - 首次注册：携带 manifest_json + mfr_signature 进行身份验证
//! - PSK 续期：携带已有 PSK token 直接续期

use prost::Message;
use tracing::{debug, error, info, warn};

use actr_protocol::{RegisterRequest, RegisterResponse};

use crate::error::{HyperError, HyperResult};

/// AIS HTTP 客户端
///
/// 封装向 AIS /register 端点发送 protobuf 请求的逻辑。
/// 所有请求使用 `application/x-protobuf` 编码。
pub struct AisClient {
    endpoint: String,
    http: reqwest::Client,
}

impl AisClient {
    /// 创建新的 AIS 客户端
    ///
    /// `endpoint` 为 AIS 基础 URL，例如 `"http://ais.example.com:8080"`。
    pub fn new(endpoint: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest::Client 构建失败（不应发生）");
        Self {
            endpoint: endpoint.into(),
            http,
        }
    }

    /// 首次注册：用 MFR manifest 认证
    ///
    /// 发送 RegisterRequest（含 manifest_json + mfr_signature），
    /// 接收 RegisterResponse。
    /// 首次注册时 AIS 会在响应中下发 PSK，供后续续期使用。
    pub async fn register_with_manifest(
        &self,
        req: RegisterRequest,
    ) -> HyperResult<RegisterResponse> {
        info!(
            endpoint = %self.endpoint,
            "首次注册：通过 MFR manifest 向 AIS 发起注册"
        );
        self.do_register(req).await
    }

    /// 续期注册：用 PSK 认证
    ///
    /// 发送 RegisterRequest（含 psk_token），
    /// 接收 RegisterResponse，返回新 credential。
    pub async fn register_with_psk(&self, req: RegisterRequest) -> HyperResult<RegisterResponse> {
        debug!(
            endpoint = %self.endpoint,
            "PSK 续期：通过现有 PSK 向 AIS 续期 credential"
        );
        self.do_register(req).await
    }

    /// 发送 POST /register 请求，通用逻辑
    ///
    /// 将 RegisterRequest protobuf 编码后 POST 到 `{endpoint}/register`，
    /// 解码响应为 RegisterResponse。
    async fn do_register(&self, req: RegisterRequest) -> HyperResult<RegisterResponse> {
        let url = format!("{}/register", self.endpoint);

        // 编码为 protobuf bytes
        let body = req.encode_to_vec();

        debug!(url = %url, body_len = body.len(), "发送 AIS 注册请求");

        let response = self
            .http
            .post(&url)
            .header("Content-Type", "application/x-protobuf")
            .header("Accept", "application/x-protobuf")
            .body(body)
            .send()
            .await
            .map_err(|e| {
                error!(url = %url, error = %e, "AIS HTTP 请求失败");
                HyperError::AisBootstrapFailed(format!("HTTP 请求失败: {e}"))
            })?;

        let status = response.status();
        if !status.is_success() {
            warn!(url = %url, status = %status, "AIS 返回非 2xx 状态码");
            return Err(HyperError::AisBootstrapFailed(format!(
                "AIS 返回错误状态码: {status}"
            )));
        }

        let bytes = response.bytes().await.map_err(|e| {
            error!(url = %url, error = %e, "读取 AIS 响应 body 失败");
            HyperError::AisBootstrapFailed(format!("读取响应 body 失败: {e}"))
        })?;

        debug!(url = %url, response_len = bytes.len(), "收到 AIS 响应");

        let resp = RegisterResponse::decode(bytes.as_ref()).map_err(|e| {
            error!(url = %url, error = %e, "解码 AIS RegisterResponse 失败");
            HyperError::AisBootstrapFailed(format!("响应 protobuf 解码失败: {e}"))
        })?;

        Ok(resp)
    }
}
