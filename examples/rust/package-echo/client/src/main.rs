//! Package Echo Client — discovers the remote echo server and calls it directly.
//!
//! Pattern: AppSide -> ActrRef.call_remote(server_id, EchoRequest)
//!          -> PeerGate -> WebRTC -> remote echo server -> EchoResponse

mod app_side;

/// Generated protobuf types from echo.proto
pub mod echo {
    include!(concat!(env!("OUT_DIR"), "/echo.rs"));
}

use std::env;
use std::path::PathBuf;

use actr_hyper::{Hyper, HyperConfig, TrustMode, init_observability};
use actr_platform_native::NativePlatformProvider;
use actr_protocol::{ActrType, RpcRequest};
use anyhow::{Context, Result};
use tracing::{error, info};

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
    info!("📡 Signaling server: {}", config.signaling_url);

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. Initialize Hyper (for credential bootstrap)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let hyper_data_dir = config.config_dir.join(".hyper");
    // Client has no guest package; trust_mode is only used for package verification
    // which is never called here. Use Development mode with a dummy key as placeholder.
    let trust_mode = TrustMode::Development {
        self_signed_pubkey: vec![0u8; 32],
    };

    let hyper = Hyper::init_with_platform(
        HyperConfig::new(&hyper_data_dir).with_trust_mode(trust_mode),
        std::sync::Arc::new(NativePlatformProvider::new()),
    )
    .await
    .context("Hyper initialization failed")?;

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 3. Register with AIS using config (no package manifest)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let ais_endpoint =
        env::var("AIS_ENDPOINT").unwrap_or_else(|_| "http://localhost:8081/ais".to_string());
    info!("🔐 Registering with AIS at {}", ais_endpoint);

    let register_ok = hyper
        .bootstrap_credential_from_config(&config, &ais_endpoint)
        .await
        .inspect_err(|e| error!("❌ AIS registration failed: {:?}", e))?;
    info!(
        "✅ AIS registration successful, ActrId: {}",
        actr_protocol::ActrIdExt::to_string_repr(&register_ok.actr_id)
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 4. Attach no-op workload, start node
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let mut node = hyper.attach_none(config).await?;
    node.inject_credential(register_ok);

    info!("🚀 Starting ActrNode...");
    let actr_ref = node.start().await.inspect_err(|e| {
        error!("❌ ActrNode start failed: {:?}", e);
    })?;
    info!(
        "✅ ActrNode started with ID: {}",
        actr_protocol::ActrIdExt::to_string_repr(actr_ref.actor_id())
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 5. Discover remote echo server
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let target_type = ActrType {
        manufacturer: "actrium".to_string(),
        name: "EchoService".to_string(),
        version: "0.2.0-beta".to_string(),
    };

    info!("🔍 Discovering echo server...");
    let mut candidates = actr_ref
        .discover_route_candidates(&target_type, 1)
        .await
        .context("Failed to discover echo server")?;

    let server_id = candidates
        .pop()
        .ok_or_else(|| anyhow::anyhow!("No echo server found"))?;
    info!(
        "🎯 Found server: {}",
        actr_protocol::ActrIdExt::to_string_repr(&server_id)
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 6. Run interactive app
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let app_side = app_side::AppSide {
        actr_ref: actr_ref.clone(),
        server_id,
    };
    app_side.run().await;

    actr_ref.shutdown();
    actr_ref.wait_for_shutdown().await;

    info!("👋 Client shut down");
    Ok(())
}
