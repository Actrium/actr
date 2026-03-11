//! Echo Real Server - 真实的 Actor 服务端
//!
//! 使用 ActorSystem 启动，通过 signaling server 注册

mod echo_service;
mod generated;

use echo_service::EchoService;
use generated::echo_actor::EchoServiceWorkload;

use actr_runtime::prelude::*;
use std::path::PathBuf;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 1. 加载配置
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // 初始化可观测性（日志/链路追踪）
    let _obs_guard = actr_runtime::init_observability(&config.observability)?;

    info!("🚀 Echo Real Server 启动");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📝 使用真实的 protobuf 注册流程");
    info!("📡 需要 signaling-server 运行在 ws://localhost:8081");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. 创建 ActorSystem
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🏗️  创建 ActorSystem...");

    let system = match ActrSystem::new(config).await {
        Ok(sys) => sys,
        Err(e) => {
            error!("❌ ActrSystem 创建失败: {:?}", e);
            return Err(e.into());
        }
    };

    info!("✅ ActrSystem 创建成功");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 3. 创建 EchoService 并附加 Workload
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("📦 创建 EchoService...");

    let echo_service = EchoService::new();
    let workload = EchoServiceWorkload::new(echo_service);
    let node = system.attach(workload);

    info!("✅ EchoService 已附加");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 4. 启动 ActrNode (连接 signaling server, 注册, 启动接收)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🚀 启动 ActrNode...");

    let actr_ref = match node.start().await {
        Ok(actr) => actr,
        Err(e) => {
            error!("❌ ActrNode 启动失败: {:?}", e);
            error!("💡 提示：请确保 signaling-server 已运行: cd signaling-server && cargo run");
            return Err(e.into());
        }
    };

    info!("✅ ActrNode 启动成功！");
    info!("🆔 Server ID: {:?}", actr_ref.actor_id());
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("🎉 Echo Server 已完全启动并注册");
    info!("📡 等待客户端连接...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 5. Wait for Ctrl+C and shutdown
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ Echo Server 已关闭");

    Ok(())
}
