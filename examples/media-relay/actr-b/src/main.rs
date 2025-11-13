//! Actr B (Receiver) - Real Actor implementation
//!
//! Receives media frames via real WebRTC P2P RPC calls from Actr A

mod generated;
mod relay_service;

use generated::relay_service_actor::RelayServiceWorkload;
use relay_service::RelayService;

use actr_runtime::prelude::*;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("🚀 Actr B (Receiver) 启动");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📝 使用真实的 ActrSystem + WebRTC P2P");
    info!("📡 需要 signaling-server 运行在 ws://localhost:8081");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // 1. Load configuration from Actr.toml
    info!("⚙️  加载配置...");

    let config = actr_config::ConfigParser::from_file("Actr.toml")?;

    info!("✅ 配置已加载");

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
