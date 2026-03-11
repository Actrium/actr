//! AIS HTTP client
//!
//! Encapsulates the logic for sending protobuf requests to the AIS `/register` endpoint.
//! Supports two registration modes:
//! - Initial registration: authenticate with manifest_json + mfr_signature
//! - PSK renewal: renew directly using an existing PSK token

use prost::Message;
use tracing::{debug, error, info, warn};

use actr_protocol::{RegisterRequest, RegisterResponse};

use crate::error::{HyperError, HyperResult};

/// AIS HTTP client
///
/// Encapsulates the logic for sending protobuf requests to the AIS /register endpoint.
/// All requests use `application/x-protobuf` encoding.
pub struct AisClient {
    endpoint: String,
    http: reqwest::Client,
}

impl AisClient {
    /// Create a new AIS client
    ///
    /// `endpoint` is the AIS base URL, e.g. `"http://ais.example.com:8080"`.
    pub fn new(endpoint: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest::Client build failed (should never happen)");
        Self {
            endpoint: endpoint.into(),
            http,
        }
    }

    /// Initial registration: authenticate with MFR manifest
    ///
    /// Sends a RegisterRequest (containing manifest_json + mfr_signature),
    /// receives a RegisterResponse.
    /// On initial registration, AIS returns a PSK in the response for subsequent renewals.
    pub async fn register_with_manifest(
        &self,
        req: RegisterRequest,
    ) -> HyperResult<RegisterResponse> {
        info!(
            endpoint = %self.endpoint,
            "initial registration: registering with AIS via MFR manifest"
        );
        self.do_register(req).await
    }

    /// Renewal registration: authenticate with PSK
    ///
    /// Sends a RegisterRequest (containing psk_token),
    /// receives a RegisterResponse with a new credential.
    pub async fn register_with_psk(&self, req: RegisterRequest) -> HyperResult<RegisterResponse> {
        debug!(
            endpoint = %self.endpoint,
            "PSK renewal: renewing credential via existing PSK"
        );
        self.do_register(req).await
    }

    /// Send POST /register request, common logic
    ///
    /// Encodes a RegisterRequest as protobuf and POSTs it to `{endpoint}/register`,
    /// then decodes the response as RegisterResponse.
    async fn do_register(&self, req: RegisterRequest) -> HyperResult<RegisterResponse> {
        let url = format!("{}/register", self.endpoint);

        // encode as protobuf bytes
        let body = req.encode_to_vec();

        debug!(url = %url, body_len = body.len(), "sending AIS register request");

        let response = self
            .http
            .post(&url)
            .header("Content-Type", "application/x-protobuf")
            .header("Accept", "application/x-protobuf")
            .body(body)
            .send()
            .await
            .map_err(|e| {
                error!(url = %url, error = %e, "AIS HTTP request failed");
                HyperError::AisBootstrapFailed(format!("HTTP request failed: {e}"))
            })?;

        let status = response.status();
        if !status.is_success() {
            warn!(url = %url, status = %status, "AIS returned non-2xx status");
            return Err(HyperError::AisBootstrapFailed(format!(
                "AIS returned error status: {status}"
            )));
        }

        let bytes = response.bytes().await.map_err(|e| {
            error!(url = %url, error = %e, "failed to read AIS response body");
            HyperError::AisBootstrapFailed(format!("failed to read response body: {e}"))
        })?;

        debug!(url = %url, response_len = bytes.len(), "received AIS response");

        let resp = RegisterResponse::decode(bytes.as_ref()).map_err(|e| {
            error!(url = %url, error = %e, "failed to decode AIS RegisterResponse");
            HyperError::AisBootstrapFailed(format!("response protobuf decode failed: {e}"))
        })?;

        Ok(resp)
    }
}
