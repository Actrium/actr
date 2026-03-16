use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::info;

mod app_side;
mod client_workload;
mod generated;

use actr_protocol::ActrType;
use actr_hyper::prelude::*;
use app_side::AppSide;
use client_workload::ClientWorkload;

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration from actr.toml
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // Initialize observability (logging/tracing) using config
    let _obs_guard = actr_hyper::init_observability(&config.observability)?;

    info!("🚀 Echo Client App - Actor-RTC Standard Pattern");
    info!("   [...] (Runtime Compatibility Negotiation)");

    let system = ActrSystem::new(config).await?;
    info!("✅ ActrSystem created");

    // prepare workload and configure server target
    let workload = ClientWorkload::new();

    info!("🌐 Discovering echo server via signaling...");
    let target_type = ActrType {
        manufacturer: "acme".to_string(),
        name: "EchoService".to_string(),
        version: "v1".to_string(),
    };

    let node = system.attach(workload.clone());

    info!("🚀 Starting ActrNode...");
    let actr_ref = node.start().await?;
    let mut candidates = actr_ref
        .discover_route_candidates(&target_type, 1)
        .await
        .context("Failed to query signaling server for echo candidates")?;

    let server_id = candidates
        .pop()
        .ok_or_else(|| anyhow::anyhow!("No echo server instances available"))?;

    info!("🎯 Target server: {:?}", server_id);
    workload.set_server_id(server_id).await;
    let local_id = actr_ref.actor_id().clone();
    info!("✅ ActrNode started with ID: {:?}", local_id);

    // run interactive app
    let app_side = AppSide {
        actr_ref: actr_ref.clone(),
    };
    app_side.run().await;

    info!("👋 Application shut down");
    Ok(())
}
