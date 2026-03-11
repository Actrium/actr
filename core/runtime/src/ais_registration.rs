//! AIS (Actor Identity Service) HTTP registration client
//!
//! Handles actor registration with the AIS endpoint on the actrix server.
//! This replaces the previous signaling-based registration flow:
//!
//! 1. POST /ais/register with protobuf RegisterRequest body
//! 2. Receive RegisterResponse with credential and TURN credentials
//! 3. Connect to signaling WebSocket with credential in URL params

use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{RegisterRequest, RegisterResponse, register_response};
use url::Url;

/// AIS registration error
#[derive(Debug, thiserror::Error)]
pub enum AisRegistrationError {
    #[error("HTTP request failed: {0}")]
    HttpError(String),
    #[error("AIS returned error: code={code}, message={message}")]
    ServerError { code: u32, message: String },
    #[error("Failed to decode response: {0}")]
    DecodeError(String),
    #[error("Missing result in response")]
    MissingResult,
}

/// Register an actor with the AIS HTTP endpoint
///
/// # Arguments
/// * `ais_base_url` - Base URL of the actrix server (e.g., `http://localhost:8081`)
/// * `request` - The protobuf RegisterRequest
/// * `realm_secret` - Optional realm secret (sent as X-Actrix-Realm-Secret header)
///
/// # Returns
/// `RegisterOk` on success, or `AisRegistrationError` on failure
pub async fn register_with_ais(
    ais_base_url: &Url,
    request: &RegisterRequest,
    realm_secret: Option<&str>,
) -> Result<register_response::RegisterOk, AisRegistrationError> {
    let register_url = format!("{}ais/register", ais_base_url.as_str());

    tracing::info!(
        "📤 Registering actor via AIS HTTP: {} (realm={})",
        register_url,
        request.realm.realm_id,
    );

    let body = request.encode_to_vec();

    let client = reqwest::Client::new();
    let mut req = client
        .post(&register_url)
        .header("content-type", "application/octet-stream")
        .body(body);

    if let Some(secret) = realm_secret {
        req = req.header("x-actrix-realm-secret", secret);
    }

    let response = req.send().await.map_err(|e| {
        AisRegistrationError::HttpError(format!("Failed to send registration request: {e}"))
    })?;

    let status = response.status();
    let rsp_bytes = response.bytes().await.map_err(|e| {
        AisRegistrationError::HttpError(format!("Failed to read response body: {e}"))
    })?;

    if !status.is_success() && rsp_bytes.is_empty() {
        return Err(AisRegistrationError::HttpError(format!(
            "AIS returned HTTP {status} with empty body"
        )));
    }

    let register_rsp = RegisterResponse::decode(&*rsp_bytes).map_err(|e| {
        AisRegistrationError::DecodeError(format!(
            "Failed to decode RegisterResponse: {e} (HTTP status: {status})"
        ))
    })?;

    match register_rsp.result {
        Some(register_response::Result::Success(ok)) => {
            tracing::info!(
                "✅ AIS registration successful: ActrId={}",
                actr_protocol::ActrIdExt::to_string_repr(&ok.actr_id),
            );
            Ok(ok)
        }
        Some(register_response::Result::Error(err)) => {
            tracing::error!(
                "❌ AIS registration failed: code={}, message={}",
                err.code,
                err.message
            );
            Err(AisRegistrationError::ServerError {
                code: err.code,
                message: err.message,
            })
        }
        None => Err(AisRegistrationError::MissingResult),
    }
}

/// Register with AIS, retrying on transient HTTP errors.
///
/// The AIS may briefly return errors if the Signer service hasn't finished
/// initializing its signing keys. This wrapper retries up to `max_attempts`
/// times with exponential backoff (starting at 2 seconds).
pub async fn register_with_ais_retry(
    ais_base_url: &Url,
    request: &RegisterRequest,
    realm_secret: Option<&str>,
    max_attempts: u32,
) -> Result<register_response::RegisterOk, AisRegistrationError> {
    let mut last_err = None;
    let mut delay = std::time::Duration::from_secs(2);

    for attempt in 1..=max_attempts {
        match register_with_ais(ais_base_url, request, realm_secret).await {
            Ok(ok) => return Ok(ok),
            Err(e) => {
                // Don't retry server-side rejections (e.g. ACL denied, bad request)
                if matches!(&e, AisRegistrationError::ServerError { .. }) {
                    return Err(e);
                }
                tracing::warn!(
                    "AIS registration attempt {attempt}/{max_attempts} failed: {e}, retrying in {:?}",
                    delay
                );
                last_err = Some(e);
                if attempt < max_attempts {
                    tokio::time::sleep(delay).await;
                    delay =
                        std::time::Duration::from_secs(delay.as_secs().saturating_mul(2).min(10));
                }
            }
        }
    }

    Err(last_err.unwrap_or(AisRegistrationError::HttpError(
        "All registration attempts failed".into(),
    )))
}
