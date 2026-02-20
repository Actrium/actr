use actrix_sdk::control::{AdminApiService, AuthService, NodeAdminServiceServer};
use anyhow::Result;
use axum::{Json, Router, response::Html, routing::get};
use platform::{
    ServiceCollector,
    config::{ActrixConfig, ControlHead},
    storage::nonce::SqliteNonceStorage,
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Build control router with absolute routes.
///
/// Control is always available and reuses the main HTTP listener.
pub async fn build_control_router(
    config: &ActrixConfig,
    service_collector: ServiceCollector,
    shutdown_tx: broadcast::Sender<()>,
) -> Result<Router> {
    match config.control.head {
        ControlHead::AdminUi => Ok(build_admin_ui_router()),
        ControlHead::GrpcApi => build_grpc_api_router(config, service_collector, shutdown_tx).await,
    }
}

fn build_admin_ui_router() -> Router {
    Router::new()
        .route("/admin", get(control_admin_ui_index))
        .route("/admin/health", get(control_admin_ui_health))
}

async fn control_admin_ui_index() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Actrix Admin</title>
    <style>
      body { font-family: ui-sans-serif, system-ui, sans-serif; margin: 2rem; color: #111; }
      h1 { margin-bottom: .5rem; }
      code { background: #f3f3f3; padding: .15rem .35rem; border-radius: 4px; }
      .card { border: 1px solid #ddd; border-radius: 10px; padding: 1rem; max-width: 760px; }
    </style>
  </head>
  <body>
    <h1>Actrix Admin UI</h1>
    <div class="card">
      <p>Control head: <code>admin_ui</code></p>
      <p>Health endpoint: <code>/admin/health</code></p>
      <p>This node reuses the main HTTP port and does not open an extra control port.</p>
    </div>
  </body>
</html>"#,
    )
}

async fn control_admin_ui_health() -> Json<serde_json::Value> {
    Json(json!({
        "service": "control",
        "head": "admin_ui",
        "status": "healthy"
    }))
}

async fn build_grpc_api_router(
    config: &ActrixConfig,
    service_collector: ServiceCollector,
    shutdown_tx: broadcast::Sender<()>,
) -> Result<Router> {
    let grpc_cfg = &config.control.grpc_api;
    let shared_secret = Arc::new(
        hex::decode(&grpc_cfg.shared_secret)
            .map_err(|e| anyhow::anyhow!("Invalid control.grpc_api.shared_secret hex: {e}"))?,
    );
    let nonce_storage = Arc::new(
        SqliteNonceStorage::new_async(&config.sqlite_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to initialize nonce storage for control: {e}"))?,
    );

    let mut service = AdminApiService::new(
        grpc_cfg.node_id.clone(),
        grpc_cfg.effective_node_name(),
        config.location_tag.clone(),
        env!("CARGO_PKG_VERSION"),
        service_collector,
    )
    .map_err(|e| anyhow::anyhow!("Failed to create control gRPC service: {e}"))?;

    let shutdown_tx_for_handler = shutdown_tx.clone();
    service = service.with_shutdown_handler(move |_graceful, _timeout, reason| {
        let shutdown_tx = shutdown_tx_for_handler.clone();
        async move {
            if let Some(reason) = reason {
                platform::recording::warn!("Control gRPC shutdown requested: {}", reason);
            } else {
                platform::recording::warn!("Control gRPC shutdown requested");
            }
            let _ = shutdown_tx.send(());
            Ok(())
        }
    });

    let authed_service = AuthService::new(
        service,
        grpc_cfg.node_id.clone(),
        shared_secret,
        nonce_storage,
        grpc_cfg.max_clock_skew_secs,
    );
    let node_admin_service = NodeAdminServiceServer::new(authed_service);

    // Primary route for tonic clients:
    // `/admin.v1.NodeAdminService/<Method>`
    //
    // Compatibility alias:
    // `/admin/grpc/admin.v1.NodeAdminService/<Method>`
    Ok(Router::new()
        .route("/admin", get(control_grpc_head_index))
        .route("/admin/health", get(control_grpc_head_health))
        .route_service(
            "/admin.v1.NodeAdminService/{*grpc_method}",
            node_admin_service.clone(),
        )
        .nest_service("/admin/grpc", node_admin_service))
}

async fn control_grpc_head_index() -> Json<serde_json::Value> {
    Json(json!({
        "service": "control",
        "head": "grpc_api",
        "grpc_methods": "/admin.v1.NodeAdminService/<Method>",
        "grpc_compat_mount": "/admin/grpc/admin.v1.NodeAdminService/<Method>"
    }))
}

async fn control_grpc_head_health() -> Json<serde_json::Value> {
    Json(json!({
        "service": "control",
        "head": "grpc_api",
        "status": "healthy"
    }))
}
