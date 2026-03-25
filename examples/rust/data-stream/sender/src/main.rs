//! DataStream Sender Example - 100% Real Implementation
//!
//! Demonstrates sending data streams using:
//! - RPC for control messages (StartTransfer, EndTransfer)
//! - DataStream API for fast path data transmission

mod file_service;
mod generated;

use actr_hyper::prelude::*;
use std::path::PathBuf;
use tracing::info;

use crate::{
    file_service::MyFileService,
    generated::{local_file::SendFileRequest, file_actor::LocalFileServiceWorkload},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load configuration from actr.toml
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;
    info!("⚙️  Configuration loaded");

    // Initialize observability (logging/tracing) using config
    let _obs_guard = actr_hyper::init_observability(&config.observability)?;

    // Build node with sender workload
    info!("🏗️  Building ActrNode...");
    let workload = LocalFileServiceWorkload::new(MyFileService::new());
    let node = unimplemented!(
        "source-defined workload examples were removed; migrate this example to a package-backed host"
    );
    info!("✅ ActrNode created");

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
