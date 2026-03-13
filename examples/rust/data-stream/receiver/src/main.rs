//! DataStream Receiver Example - 100% Real Implementation
//!
//! Demonstrates receiving data streams using:
//! - RPC for control messages (StartTransfer, EndTransfer)
//! - DataStream API for fast path data transmission

mod file_transfer_service;
mod generated;

use file_transfer_service::FileTransferService;
use generated::file_transfer_actor::FileTransferServiceWorkload;

use actr_hyper::prelude::*;
use std::path::PathBuf;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration from actr.toml
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // Initialize observability (logging/tracing) using config
    let _obs_guard = actr_hyper::init_observability(&config.observability)?;

    info!("🚀 DataStream Receiver starting - 100% Real Implementation");
    info!("📋 Config: type={}", config.package.actr_type.name);

    // Create ActrSystem
    info!("🏗️  Creating ActrSystem...");
    let system = match ActrSystem::new(config).await {
        Ok(sys) => sys,
        Err(e) => {
            error!("❌ ActrSystem creation failed: {:?}", e);
            return Err(e.into());
        }
    };
    info!("✅ ActrSystem created");

    // Create service and wrap in Workload
    let service = FileTransferService::new();
    let workload = FileTransferServiceWorkload::new(service);

    // Attach workload
    let node = system.attach(workload);

    // Start node
    info!("🚀 Starting ActrNode...");
    let actr_ref = match node.start().await {
        Ok(actr) => actr,
        Err(e) => {
            error!("❌ ActrNode start failed: {:?}", e);
            error!("💡 Tip: Make sure signaling-server is running");
            return Err(e.into());
        }
    };

    info!("✅ ActrNode started! Actor ID: {:?}", actr_ref.actor_id());
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("🎉 Receiver ready to accept file transfers");
    info!("📡 Listening for RPC requests...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Wait for Ctrl+C
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ Receiver shut down");
    Ok(())
}
