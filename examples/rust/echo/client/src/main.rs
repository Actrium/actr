//! Echo Client — demonstrates calling the Echo server
//!
//! Always runs in Native mode. Discovers the Echo server via signaling,
//! then sends user messages and prints the echoed replies.

mod app_side;
mod client_workload;
mod generated;

use actr_hyper::prelude::*;
use actr_protocol::ActrType;
use anyhow::{Context, Result};
use app_side::AppSide;
use client_workload::ClientWorkload;
use std::path::PathBuf;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration
    let config_file = std::env::args()
        .skip_while(|a| a != "--config")
        .nth(1)
        .unwrap_or_else(|| "actr.example.toml".to_string());

    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(&config_file);
    println!("DEBUG config_path: {:?}", config_path);
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // Initialize observability
    let _obs_guard = actr_hyper::init_observability(&config.observability)?;

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🚀 Echo Client — Native / Process Mode Demo");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let system = ActrSystem::new(config).await?;
    info!("✅ ActrSystem created");

    let workload = ClientWorkload::new();
    let target_type = ActrType {
        manufacturer: "acme".to_string(),
        name: "EchoServer".to_string(),
        version: "v1".to_string(),
    };

    let node = system.attach(workload.clone());

    info!("🚀 Starting ActrNode...");
    let actr_ref = node.start().await?;
    info!("✅ ActrNode started with ID: {:?}", actr_ref.actor_id());

    // Discover server
    info!("🌐 Discovering echo server via signaling...");
    let mut candidates = actr_ref
        .discover_route_candidates(&target_type, 1)
        .await
        .context("Failed to discover echo server")?;

    let server_id = candidates
        .pop()
        .ok_or_else(|| anyhow::anyhow!("No echo server instances available"))?;

    info!("🎯 Target server: {:?}", server_id);
    workload.set_server_id(server_id).await;

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("✅ Connected! Type messages to echo (or 'quit' to exit):");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Run interactive loop
    let app_side = AppSide {
        actr_ref: actr_ref.clone(),
    };
    app_side.run().await;

    info!("👋 Client shut down");
    Ok(())
}
