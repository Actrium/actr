//! Actr B (Receiver) - Real Actor implementation
//!
//! Receives media frames via real WebRTC P2P RPC calls from Actr A

mod generated;
mod media_relay_service;

use generated::relay_service_actor::RelayServiceWorkload;
use media_relay_service::RelayService;

use actr_runtime::prelude::*;
use std::path::PathBuf;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Load configuration from Actr.toml
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // Initialize observability (logging/tracing) using config
    let _obs_guard = actr_runtime::init_observability(&config.observability)?;

    info!("🚀 Actr B (Receiver) 启动");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📝 使用真实的 ActrSystem + WebRTC P2P");
    info!("📡 需要 signaling-server 运行在 ws://localhost:8081");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // 2. Create ActrSystem
    info!("🏗️  创建 ActrSystem...");
    let system = ActrSystem::new(config).await?;
    info!("✅ ActrSystem 创建成功");

    // 3. Create RelayService and attach Workload
    info!("📦 创建 RelayService...");

    let relay_service = RelayService::new();
    let workload = RelayServiceWorkload::new(relay_service);
    let node = system.attach(workload);

    info!("✅ RelayService 已附加");

    // 4. Start ActrNode (connect to signaling, register, start receiving)
    info!("🚀 启动 ActrNode...");
    let actr_ref = node.start().await?;
    info!("✅ ActrNode 启动成功！");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("🎉 Actr B 已完全启动并注册到 signaling server");
    info!("📥 等待 Actr A 发送媒体帧...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // 5. Wait for Ctrl+C and shutdown
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ Actr B 已关闭");

    Ok(())
}
