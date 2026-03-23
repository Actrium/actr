//! Package Echo Client — discovers and communicates with the package-backed echo server
//!
//! Pattern: AppSide -> RuntimeContext.call(Dest::Actor(server_id), EchoRequest)
//!          -> package echo server -> response

mod app_side;

/// Generated protobuf types from echo.proto
pub mod echo {
    include!(concat!(env!("OUT_DIR"), "/echo.rs"));
}

use actr_framework::Context as _;
use actr_hyper::{ActrSystem, AisClient, init_observability};
use actr_protocol::RpcRequest;
use actr_protocol::{ActrType, RegisterRequest, register_response};
use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::info;

use crate::echo::{EchoRequest, EchoResponse};

impl RpcRequest for EchoRequest {
    type Response = EchoResponse;

    fn route_key() -> &'static str {
        "echo.EchoService.Echo"
    }

    fn payload_type() -> actr_protocol::PayloadType {
        actr_protocol::PayloadType::RpcReliable
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 1. Load configuration
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    let _obs_guard = init_observability(&config.observability)?;

    info!("🚀 Package Echo Client starting");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. Register with AIS HTTP to obtain credential
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let ais_endpoint =
        std::env::var("AIS_ENDPOINT").unwrap_or_else(|_| "http://localhost:8081/ais".to_string());
    info!("🔐 Registering with AIS at {}", ais_endpoint);

    let ais = AisClient::new(&ais_endpoint);
    let register_req = RegisterRequest {
        actr_type: config.actr_type().clone(),
        realm: config.realm.clone(),
        service_spec: config.calculate_service_spec(),
        acl: config.acl.clone(),
        service: None,
        ws_address: None,
        manifest_raw: None,
        mfr_signature: None,
        psk_token: None,
        target: None,
    };

    let ais_response = ais
        .register_with_manifest(register_req)
        .await
        .context("AIS registration failed")?;

    let register_ok = match ais_response.result {
        Some(register_response::Result::Success(ok)) => {
            info!(
                "✅ AIS registration successful, ActrId: {}",
                actr_protocol::ActrIdExt::to_string_repr(&ok.actr_id)
            );
            ok
        }
        Some(register_response::Result::Error(e)) => {
            anyhow::bail!(
                "AIS registration rejected: code={}, message={}",
                e.code,
                e.message
            );
        }
        None => {
            anyhow::bail!("AIS response missing result");
        }
    };

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 3. Create ActrSystem and start node
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let system = ActrSystem::new(config).await?;
    info!("✅ ActrSystem created");

    let mut node = system.attach_shell();

    // Inject AIS-issued credential so start() can authenticate with signaling
    node.inject_credential(register_ok);

    info!("🚀 Starting ActrNode...");
    let actr_ref = node.start().await?;
    info!(
        "✅ ActrNode started with ID: {}",
        actr_protocol::ActrIdExt::to_string_repr(actr_ref.actor_id())
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 4. Discover the local-package-backed echo server
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🌐 Discovering local echo-actr service via signaling...");
    let target_type = ActrType {
        manufacturer: "actrium".to_string(),
        name: "EchoService".to_string(),
        version: "0.1.0".to_string(),
    };

    let app_ctx = actr_ref.app_context().await;

    let server_id = app_ctx
        .discover_route_candidate(&target_type)
        .await
        .context("Failed to discover local package echo server")?;

    info!(
        "🎯 Target server: {}",
        actr_protocol::ActrIdExt::to_string_repr(&server_id)
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 5. Run interactive app
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let app_side = app_side::AppSide { app_ctx, server_id };
    app_side.run().await;

    actr_ref.shutdown();
    actr_ref.wait_for_shutdown().await;

    info!("👋 Client shut down");
    Ok(())
}
