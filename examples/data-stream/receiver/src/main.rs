//! DataStream Receiver Example - 100% Real Implementation
//!
//! Demonstrates receiving data streams using:
//! - RPC for control messages (StartTransfer, EndTransfer)
//! - DataStream API for fast path data transmission

mod file_transfer_service;
mod generated;

use file_transfer_service::FileTransferService;
use generated::file_transfer_service_actor::FileTransferServiceWorkload;

use actr_protocol::{ActrType, Realm};
use actr_runtime::prelude::*;
use std::collections::HashMap;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("🚀 DataStream Receiver starting - 100% Real Implementation");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📊 Waiting for file transfer requests...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Create config
    let config = actr_config::Config {
        package: actr_config::PackageInfo {
            name: "file-transfer-receiver".to_string(),
            actr_type: ActrType {
                manufacturer: "acme".to_string(),
                name: "file-transfer.FileTransferService".to_string(),
            },
            description: Some("File transfer receiver using DataStream API".to_string()),
            authors: vec![],
            license: Some("Apache-2.0".to_string()),
        },
        exports: vec![],
        dependencies: vec![],
        signaling_url: url::Url::parse("ws://localhost:8081/signaling/ws")?,
        realm: Realm { realm_id: 0 },
        visible_in_discovery: true,
        acl: None,
        mailbox_path: None,
        tags: vec!["dev".to_string(), "example".to_string()],
        scripts: HashMap::new(),
    };

    // Create ActrSystem
    info!("🏗️  Creating ActrSystem...");
    info!("📋 Config: type={}", config.package.actr_type.name);
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
