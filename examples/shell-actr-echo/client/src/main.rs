use anyhow::Result;
use tracing::info;
use std::collections::HashMap;

mod generated;
mod client_workload;
mod app_side;

use actr_protocol::{ActrType, Realm, ActrId};
use actr_runtime::prelude::*;
use client_workload::ClientWorkload;
use app_side::AppSide;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing with env filter support (RUST_LOG)
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("🚀 Echo Client App - Actor-RTC Standard Pattern");

    // minimal config
    let config = actr_config::Config {
        package: actr_config::PackageInfo {
            name: "echo-real-client-app".to_string(),
            actr_type: ActrType {
                manufacturer: "acme".to_string(),
                name: "echo-client-app".to_string(),
            },
            description: Some("Echo Client App".to_string()),
            authors: vec![],
            license: Some("Apache-2.0".to_string()),
        },
        exports: vec![],
        dependencies: vec![],
        signaling_url: url::Url::parse("ws://localhost:8081/signaling/ws")?,
        realm: Realm { realm_id: 0 },
        visible_in_discovery: true,
        acl: None,
        mailbox_path: None,  // Use in-memory database
        tags: vec!["dev".to_string(), "client".to_string()],
        scripts: HashMap::new(),
    };

    let system = ActrSystem::new(config).await?;
    info!("✅ ActrSystem created");

    // prepare workload and configure server target
    let workload = ClientWorkload::new();
    let server_id = ActrId {
        realm: Realm { realm_id: 0 },
        serial_number: 1000,
        r#type: ActrType {
            manufacturer: "acme".to_string(),
            name: "echo.EchoService".to_string(),
        },
    };
    workload.set_server_id(server_id.clone()).await;
    info!("🎯 Target server: {:?}", server_id);

    let node = system.attach(workload);

    info!("🚀 Starting ActrNode...");
    let actr_ref = node.start().await?;
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
