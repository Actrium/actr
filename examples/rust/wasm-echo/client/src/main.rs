//! WASM Echo Client — discovers and communicates with the WASM echo server
//!
//! Pattern: AppSide → ActrRef.call(EchoRequest) → ClientWorkload.dispatch
//!          → ctx.call(Dest::Actor(server_id), req) → WASM echo server → response

mod app_side;
mod client_workload;

/// Generated protobuf types from echo.proto
pub mod echo {
    include!(concat!(env!("OUT_DIR"), "/echo.rs"));
}

use actr_hyper::{ActrSystem, AisClient, init_observability};
use actr_protocol::{ActrType, RegisterRequest, register_response};
use anyhow::{Context, Result};
use client_workload::ClientWorkload;
use std::path::PathBuf;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 1. Load configuration
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    let _obs_guard = init_observability(&config.observability)?;

    info!("🚀 WASM Echo Client starting");

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
        manifest_json: None,
        mfr_signature: None,
        psk_token: None,
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

    let workload = ClientWorkload::new();
    let mut node = system.attach(workload.clone());

    // Inject AIS-issued credential so start() can authenticate with signaling
    node.inject_credential(register_ok);

    info!("🚀 Starting ActrNode...");
    let actr_ref = node.start().await?;
    info!("✅ ActrNode started with ID: {:?}", actr_ref.actor_id());

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 4. Discover WASM echo server
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🌐 Discovering WASM echo server via signaling...");
    let target_type = ActrType {
        manufacturer: "acme".to_string(),
        name: "WasmEchoService".to_string(),
        version: "0.1.0".to_string(),
    };

    let mut candidates = actr_ref
        .discover_route_candidates(&target_type, 1)
        .await
        .context("Failed to discover WASM echo server")?;

    let server_id = candidates
        .pop()
        .ok_or_else(|| anyhow::anyhow!("No WASM echo server instances found"))?;

    info!("🎯 Target server: {:?}", server_id);
    workload.set_server_id(server_id).await;

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 5. Run interactive app
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let app_side = app_side::AppSide {
        actr_ref: actr_ref.clone(),
    };
    app_side.run().await;

    info!("👋 Client shut down");
    Ok(())
}
