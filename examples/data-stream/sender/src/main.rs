//! DataStream Sender Example - 100% Real Implementation
//!
//! Demonstrates sending data streams using:
//! - RPC for control messages (StartTransfer, EndTransfer)
//! - DataStream API for fast path data transmission

mod file_service;
mod generated;

use actr_runtime::prelude::*;
use std::path::PathBuf;
use tracing::info;

use crate::{
    file_service::MyFileService,
    generated::{local_file::SendFileRequest, local_file_service_actor::LocalFileServiceWorkload},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load configuration from Actr.toml
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;
    info!("⚙️  Configuration loaded");

    // Initialize observability (logging/tracing) using config
    let _obs_guard = actr_runtime::init_observability(&config.observability)?;

    // Create ActrSystem
    info!("🏗️  Creating ActrSystem...");
    let system = ActrSystem::new(config).await?;
    info!("✅ ActrSystem created");

    // Attach sender workload
    let workload = LocalFileServiceWorkload::new(MyFileService::new());
    let node = system.attach(workload);

    info!("🚀 Starting ActrNode...");
    let actr_ref = node.start().await?;
    info!("✅ ActrNode started! Actor ID: {:?}", actr_ref.actor_id());

    let response = actr_ref
        .call(SendFileRequest {
            filename: "test-file.txt".to_string(),
        })
        .await?;
    info!("✅ Response: {:?}", response);

    Ok(())
}
