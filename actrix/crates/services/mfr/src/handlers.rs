//! MFR (Manufacturer Registry) HTTP handlers

use crate::{MfrError, MfrManager, manager::PublishRequest, model::MfrStatus};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct MfrState {
    pub manager: Arc<MfrManager>,
}

pub fn create_router(state: MfrState) -> Router {
    Router::new()
        // Public registration flow
        .route("/apply", post(apply))
        .route("/{id}/verify", post(verify_github))
        .route("/{id}/challenge", get(get_challenge))
        .route("/{id}/status", get(get_status))
        .route("/resolve/{name}", get(resolve_by_name))
        // Package management
        .route("/pkg/publish", post(publish_package))
        .route("/pkg", get(list_packages))
        .route("/pkg/{id}/revoke", post(revoke_package))
        // Admin endpoints
        .route("/admin/list", get(admin_list))
        .route("/admin/{id}/approve", post(admin_approve))
        .route("/admin/{id}/suspend", post(admin_suspend))
        .route("/admin/{id}/reinstate", post(admin_reinstate))
        .route("/admin/{id}", delete(admin_delete))
        .with_state(state)
}

// --- Request / Response types ---

#[derive(Deserialize)]
struct ApplyRequest {
    github_login: String,
    contact: Option<String>,
}

#[derive(Serialize)]
struct ApplyResponse {
    mfr_id: i64,
    challenge_token: String,
    expires_at: i64,
    verify_file: String,
    instructions: String,
}

#[derive(Deserialize)]
struct ListPackagesQuery {
    mfr: Option<String>,
}

// --- Handlers ---

async fn apply(
    State(s): State<MfrState>,
    Json(req): Json<ApplyRequest>,
) -> Result<Json<ApplyResponse>, MfrError> {
    let (mfr, challenge) = s
        .manager
        .apply(&req.github_login, req.contact.as_deref())
        .await?;
    let filename = crate::github::verify_filename(s.manager.domain());
    Ok(Json(ApplyResponse {
        mfr_id: mfr.id,
        challenge_token: challenge.token.clone(),
        expires_at: challenge.expires_at,
        verify_file: filename.clone(),
        instructions: format!(
            "Create a public GitHub repo '{}/{}' with a file '{}' containing the token above, then call POST /{}/verify",
            mfr.name,
            crate::github::VERIFY_REPO,
            filename,
            mfr.id,
        ),
    }))
}

async fn verify_github(
    State(s): State<MfrState>,
    Path(id): Path<i64>,
) -> Result<Json<crate::manager::MfrKeychain>, MfrError> {
    let keychain = s.manager.verify_github(id).await?;
    Ok(Json(keychain))
}

async fn get_challenge(
    State(s): State<MfrState>,
    Path(id): Path<i64>,
) -> Result<Json<ApplyResponse>, MfrError> {
    let mfr = s.manager.get_status(id).await?;
    let challenge = s.manager.get_challenge(id).await?;
    let filename = crate::github::verify_filename(s.manager.domain());
    Ok(Json(ApplyResponse {
        mfr_id: mfr.id,
        challenge_token: challenge.token,
        expires_at: challenge.expires_at,
        verify_file: filename.clone(),
        instructions: format!(
            "Create a public GitHub repo '{}/{}' with a file '{}' containing the token above, then call POST /{}/verify",
            mfr.name,
            crate::github::VERIFY_REPO,
            filename,
            mfr.id,
        ),
    }))
}

async fn get_status(
    State(s): State<MfrState>,
    Path(id): Path<i64>,
) -> Result<Json<crate::model::Manufacturer>, MfrError> {
    Ok(Json(s.manager.get_status(id).await?))
}

async fn resolve_by_name(
    State(s): State<MfrState>,
    Path(name): Path<String>,
) -> Result<Json<crate::manager::MfrPublicInfo>, MfrError> {
    Ok(Json(s.manager.resolve_by_name(&name).await?))
}

async fn publish_package(
    State(s): State<MfrState>,
    Json(req): Json<PublishRequest>,
) -> Result<Json<crate::model::ActrPackage>, MfrError> {
    Ok(Json(s.manager.publish_package(req).await?))
}

async fn list_packages(
    State(s): State<MfrState>,
    Query(q): Query<ListPackagesQuery>,
) -> Result<Json<Vec<crate::model::ActrPackage>>, MfrError> {
    Ok(Json(s.manager.list_packages(q.mfr.as_deref()).await?))
}

async fn revoke_package(
    State(s): State<MfrState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, MfrError> {
    s.manager.revoke_package(id).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn admin_list(
    State(s): State<MfrState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<crate::model::Manufacturer>>, MfrError> {
    let status = q.get("status").and_then(|v| v.parse::<MfrStatus>().ok());
    Ok(Json(s.manager.admin_list(status).await?))
}

async fn admin_approve(
    State(s): State<MfrState>,
    Path(id): Path<i64>,
) -> Result<Json<crate::manager::MfrKeychain>, MfrError> {
    Ok(Json(s.manager.admin_approve(id).await?))
}

async fn admin_suspend(
    State(s): State<MfrState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, MfrError> {
    s.manager.admin_suspend(id).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn admin_reinstate(
    State(s): State<MfrState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, MfrError> {
    s.manager.admin_reinstate(id).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn admin_delete(
    State(s): State<MfrState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, MfrError> {
    s.manager.admin_delete(id).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}
