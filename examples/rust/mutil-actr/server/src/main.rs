//! Echo Real Server - real Actor server
//!
//! using/use ActorSystem start，via/through signaling server register

mod echo_service;
mod generated;

use echo_service::EchoService;
use generated::echo_actor::EchoServiceWorkload;

use actr_hyper::prelude::*;
use std::path::PathBuf;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 1. loadconfig
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // initialize[...]（log/[...]）
    let _obs_guard = actr_hyper::init_observability(&config.observability)?;

    info!("🚀 Echo Real Server start");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📝 using/usereal protobuf register[...]");
    info!("📡 need/require signaling-server [...] ws://localhost:8081");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. create ActorSystem
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🏗️  create ActorSystem...");

    let system = match ActrSystem::new(config).await {
        Ok(sys) => sys,
        Err(e) => {
            error!("❌ ActrSystem createfailed: {:?}", e);
            return Err(e.into());
        }
    };

    info!("✅ ActrSystem createsuccess");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 3. create EchoService [...]attach Workload
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("📦 create EchoService...");

    let echo_service = EchoService::new();
    let workload = EchoServiceWorkload::new(echo_service);
    let node = system.attach(workload);

    info!("✅ EchoService attached");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 4. start ActrNode (connection signaling server, register, startreceive)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🚀 start ActrNode...");

    let actr_ref = match node.start().await {
        Ok(actr) => actr,
        Err(e) => {
            error!("❌ ActrNode startfailed: {:?}", e);
            error!("💡 tip/hint：please ensure signaling-server already[...]: cd signaling-server && cargo run");
            return Err(e.into());
        }
    };

    info!("✅ ActrNode startsuccess！");
    info!("🆔 Server ID: {:?}", actr_ref.actor_id());
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("🎉 Echo Server alreadyfully/completelystart[...]register");
    info!("📡 waiting/wait forclientconnection...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 5. Wait for Ctrl+C and shutdown
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ Echo Server closed");

    Ok(())
}
