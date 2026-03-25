//! Echo Server — minimal echo service example
//!
//! Demonstrates the standard Actor startup flow:
//! config → package-backed `Hyper.attach_package(...)` → node.start() → serve

mod echo_service;
mod generated;

use echo_service::EchoService;
use generated::echo_actor::EchoServiceWorkload;

use actr::prelude::*;
use std::path::PathBuf;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Load config
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr::config::ConfigParser::from_file(&config_path)?;

    let _obs_guard = init_observability(&config.observability)?;

    info!("Echo Server starting");

    // 2. Build ActrNode with EchoService workload
    let workload = EchoServiceWorkload::new(EchoService);
    let node = unimplemented!(
        "source-defined workload examples were removed; migrate this example to a package-backed host"
    );

    // 3. Start ActrNode (connect signaling, register, begin serving)
    let actr_ref = match node.start().await {
        Ok(r) => r,
        Err(e) => {
            error!("ActrNode start failed: {:?}", e);
            return Err(e.into());
        }
    };

    info!(id = ?actr_ref.actor_id(), "Echo Server ready, waiting for requests...");

    // 4. Wait for Ctrl+C
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("Echo Server stopped");
    Ok(())
}
