//! Actr B (Receiver) - Real Actor implementation
//!
//! Receives media frames via real WebRTC P2P RPC calls from Actr A

mod generated;
mod media_relay_service;

use generated::media_relay_actor::RelayServiceWorkload;
use media_relay_service::RelayService;

use actr_hyper::prelude::*;
use std::path::PathBuf;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Load configuration from actr.toml
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // Initialize observability (logging/tracing) using config
    let _obs_guard = actr_hyper::init_observability(&config.observability)?;

    info!("🚀 Actr B (Receiver) start");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📝 using/usereal ActrNode + WebRTC P2P");
    info!("📡 need/require signaling-server [...] ws://localhost:8081");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // 2. Create RelayService and build node
    info!("📦 create RelayService...");

    let relay_service = RelayService::new();
    let workload = RelayServiceWorkload::new(relay_service);
    let node = unimplemented!(
        "source-defined workload examples were removed; migrate this example to a package-backed host"
    );

    info!("✅ ActrNode createsuccess");

    // 3. Start ActrNode (connect to signaling, register, start receiving)
    info!("🚀 start ActrNode...");
    let actr_ref = node.start().await?;
    info!("✅ ActrNode startsuccess！");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("🎉 Actr B alreadyfully/completelystart[...]register[...] signaling server");
    info!("📥 waiting/wait for Actr A send[...]...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // 4. Wait for Ctrl+C and shutdown
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ Actr B closed");

    Ok(())
}
