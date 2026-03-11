use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::info;

mod app_side;
mod client_workload;
mod generated;

use actr_protocol::ActrType;
use actr_runtime::prelude::*;
use app_side::AppSide;
use client_workload::ClientWorkload;

#[tokio::main]
async fn main() -> Result<()> {
    // loadconfig
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // initialize[...]
    let _obs_guard = actr_runtime::init_observability(&config.observability)?;

    info!("🚀 WS Echo Client App");
    info!("   via/throughsignalingservice[...]discoverserver WebSocket address[...]direct connection");

    let system = ActrSystem::new(config).await?;
    info!("✅ ActrSystem createsuccess");

    // [...] workload
    let workload = ClientWorkload::new();

    // via/throughsignalingservice[...]discover WsEchoService
    let target_type = ActrType {
        manufacturer: "acme".to_string(),
        name: "WsEchoService".to_string(),
        version: "1.0.0".to_string(),
    };

    let node = system.attach(workload.clone());

    info!("🚀 start ActrNode...");
    let actr_ref = node.start().await?;

    info!("🌐 via/throughsignalingservice[...]discover WsEchoService...");
    let mut candidates = actr_ref
        .discover_route_candidates(&target_type, 1)
        .await
        .context("[...]signalingservice[...]discover WsEchoService [...]")?;

    let server_id = candidates
        .pop()
        .ok_or_else(|| anyhow::anyhow!("[...] WsEchoService [...]"))?;

    info!("🎯 [...]service[...]: {:?}", server_id);
    info!("🔌 alreadyvia/throughsignalingdiscover WebSocket address，[...]willusing/use WebSocket direct connection");
    workload.set_server_id(server_id).await;

    let local_id = actr_ref.actor_id().clone();
    info!("✅ ActrNode startsuccess，ID: {:?}", local_id);

    // [...]
    let app_side = AppSide {
        actr_ref: actr_ref.clone(),
    };
    app_side.run().await;

    info!("👋 [...]already[...]");
    Ok(())
}
