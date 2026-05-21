//! HTTP handlers for the mock actrix server.
//!
//! Covers the REST endpoints that `actr-hyper` (`AisClient`, `MfrCertCache`)
//! and the `actr` CLI (`pkg publish`) talk to, plus admin seeding routes
//! used by integration test scripts.

use std::sync::Arc;

use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{RegisterRequest, RegisterResponse, register_response};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

use crate::signaling;
use crate::state::{MfrEntry, MockState, PackageEntry, RegisteredActor};

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

pub async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

// ---------------------------------------------------------------------------
// AIS `/register` — protobuf in, protobuf out.
// ---------------------------------------------------------------------------

/// `POST /register` and `POST /ais/register`.
///
/// Shares the credential/ID allocation path with the WebSocket signaling
/// handler via [`signaling::build_register_ok`].
pub async fn register_handler(
    State(state): State<Arc<MockState>>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let req = RegisterRequest::decode(body.as_ref())
        .map_err(|e| ApiError::bad_request(format!("protobuf decode failed: {e}")))?;

    // Lazily register the realm so callers don't have to pre-seed.
    let realm_id = req.realm.realm_id;
    state
        .add_realm(realm_id, format!("mock-realm-{realm_id}"))
        .await;

    let register_ok = signaling::build_register_ok(&req, &state).await;

    // Track the HTTP-registered actor in the WS registry so route discovery
    // works once the actor opens its WebSocket with `?actor_id=...`. We
    // record an empty `client_id` placeholder that the WS upgrade handler
    // fills in when the peer connects.
    state.registry.write().await.push(RegisteredActor {
        actr_id: register_ok.actr_id.clone(),
        actr_type: req.actr_type.clone(),
        client_id: String::new(),
        ws_address: req.ws_address.clone(),
        service_spec: req.service_spec.clone(),
    });

    let response = RegisterResponse {
        result: Some(register_response::Result::Success(register_ok.clone())),
    };

    tracing::info!(
        serial = register_ok.actr_id.serial_number,
        manufacturer = req.actr_type.manufacturer,
        name = req.actr_type.name,
        "mock-actrix: registered actor (http)"
    );

    let bytes = response.encode_to_vec();
    Ok((
        StatusCode::OK,
        [("content-type", "application/x-protobuf")],
        bytes,
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// MFR verifying key — `GET /mfr/{name}/verifying_key?key_id=X`
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct VerifyingKeyQuery {
    // AIS allows callers to pin a specific key_id; we don't key-rotate in the
    // mock but we accept the parameter for protocol compatibility.
    pub key_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct VerifyingKeyResponse {
    public_key: String,
}

pub async fn verifying_key_handler(
    State(state): State<Arc<MockState>>,
    Path(name): Path<String>,
    Query(_query): Query<VerifyingKeyQuery>,
) -> Result<Response, ApiError> {
    let key = match state.mfr_verifying_key(&name).await {
        Some(k) => k,
        None => {
            tracing::warn!(manufacturer = %name, "mock-actrix: MFR not registered");
            return Err(ApiError::not_found(format!(
                "manufacturer '{name}' not registered"
            )));
        }
    };

    let body = VerifyingKeyResponse {
        public_key: base64::engine::general_purpose::STANDARD.encode(key.to_bytes()),
    };
    Ok(Json(body).into_response())
}

// ---------------------------------------------------------------------------
// Publish: nonce + publish (used by `actr pkg publish`).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct NonceRequest {
    pub manufacturer: String,
}

#[derive(Debug, Serialize)]
pub struct NonceResponse {
    pub nonce: String,
}

pub async fn publish_nonce_handler(
    State(state): State<Arc<MockState>>,
    Json(req): Json<NonceRequest>,
) -> Json<NonceResponse> {
    let bytes: [u8; 32] = rand::random();
    let nonce = base64::engine::general_purpose::STANDARD.encode(bytes);
    state.nonces.issue(&req.manufacturer, nonce.clone()).await;
    Json(NonceResponse { nonce })
}

#[derive(Debug, Deserialize)]
pub struct PublishRequest {
    pub manufacturer: String,
    pub name: String,
    pub version: String,
    pub target: String,
    pub manifest: String,
    pub signature: String,
    #[serde(default)]
    pub proto_files: Option<serde_json::Value>,
    pub nonce: String,
    pub nonce_sig: String,
}

#[derive(Debug, Serialize)]
pub struct PublishResponse {
    pub id: u64,
    pub type_str: String,
    pub status: String,
}

pub async fn publish_handler(
    State(state): State<Arc<MockState>>,
    Json(req): Json<PublishRequest>,
) -> Result<Json<PublishResponse>, ApiError> {
    // Consume the nonce.
    if !state.nonces.take(&req.manufacturer, &req.nonce).await {
        return Err(ApiError::bad_request(
            "publish nonce mismatch or already consumed".to_string(),
        ));
    }

    // Note: the mock deliberately does not verify the ed25519 signature
    // against the MFR's registered pubkey. Real actrix does; leaving this as
    // a trust-on-first-publish flow keeps the mock simple for CLI e2e tests.
    let _ = req.nonce_sig;
    let _ = req.proto_files;

    let entry = PackageEntry {
        manufacturer: req.manufacturer.clone(),
        name: req.name.clone(),
        version: req.version.clone(),
        target: req.target.clone(),
        manifest_raw: req.manifest,
        signature_b64: req.signature,
    };
    state.add_package(entry).await;

    let id = state.packages.read().await.len() as u64;
    let type_str = format!("{}:{}:{}", req.manufacturer, req.name, req.version);

    tracing::info!(%type_str, target = %req.target, "mock-actrix: package published");

    Ok(Json(PublishResponse {
        id,
        type_str,
        status: "active".into(),
    }))
}

// ---------------------------------------------------------------------------
// Admin seeding — replaces register.sh sqlite3 INSERTs.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AdminRealmRequest {
    pub id: u32,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdminOk {
    pub ok: bool,
}

pub async fn admin_realm_handler(
    State(state): State<Arc<MockState>>,
    Json(req): Json<AdminRealmRequest>,
) -> Json<AdminOk> {
    state
        .add_realm(
            req.id,
            req.name.unwrap_or_else(|| format!("realm-{}", req.id)),
        )
        .await;
    Json(AdminOk { ok: true })
}

#[derive(Debug, Deserialize)]
pub struct AdminMfrRequest {
    pub name: String,
    pub pubkey_b64: String,
    #[serde(default)]
    pub contact: Option<String>,
}

pub async fn admin_mfr_handler(
    State(state): State<Arc<MockState>>,
    Json(req): Json<AdminMfrRequest>,
) -> Result<Json<AdminOk>, ApiError> {
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(req.pubkey_b64.trim())
        .map_err(|e| ApiError::bad_request(format!("pubkey base64 decode failed: {e}")))?;
    let arr: [u8; 32] = key_bytes.try_into().map_err(|v: Vec<u8>| {
        ApiError::bad_request(format!("pubkey must be 32 bytes, got {}", v.len()))
    })?;
    let verifying_key = VerifyingKey::from_bytes(&arr)
        .map_err(|e| ApiError::bad_request(format!("invalid ed25519 pubkey: {e}")))?;

    state.mfrs.write().await.insert(
        req.name.clone(),
        MfrEntry {
            name: req.name.clone(),
            verifying_key,
            contact: req.contact,
        },
    );
    tracing::info!(name = %req.name, "mock-actrix: MFR seeded via /admin/mfr");
    Ok(Json(AdminOk { ok: true }))
}

#[derive(Debug, Deserialize)]
pub struct AdminPackageRequest {
    pub manufacturer: String,
    pub name: String,
    pub version: String,
    pub target: String,
    #[serde(default)]
    pub manifest: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
}

pub async fn admin_package_handler(
    State(state): State<Arc<MockState>>,
    Json(req): Json<AdminPackageRequest>,
) -> Json<AdminOk> {
    state
        .add_package(PackageEntry {
            manufacturer: req.manufacturer,
            name: req.name,
            version: req.version,
            target: req.target,
            manifest_raw: req.manifest.unwrap_or_default(),
            signature_b64: req.signature.unwrap_or_default(),
        })
        .await;
    Json(AdminOk { ok: true })
}

#[derive(Debug, Serialize)]
pub struct AdminStateSnapshot {
    pub realms: Vec<u32>,
    pub mfrs: Vec<String>,
    pub packages: Vec<String>,
}

pub async fn admin_state_handler(State(state): State<Arc<MockState>>) -> Json<AdminStateSnapshot> {
    let realms: Vec<u32> = state.realms.read().await.keys().copied().collect();
    let mfrs: Vec<String> = state.mfrs.read().await.keys().cloned().collect();
    let packages: Vec<String> = state
        .packages
        .read()
        .await
        .iter()
        .map(|p| format!("{}:{}:{}@{}", p.manufacturer, p.name, p.version, p.target))
        .collect();
    Json(AdminStateSnapshot {
        realms,
        mfrs,
        packages,
    })
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    fn not_found(message: String) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.message });
        (self.status, Json(body)).into_response()
    }
}
