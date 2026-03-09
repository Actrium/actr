//! WS Echo Server - 使用 WebSocket 通道的 Actor 服务端
//!
//! 通过配置 [system.websocket] 启用 WebSocket 直连模式：
//! - listen_port: 在此端口监听入站 WebSocket 连接
//! - advertised_host: 注册到信令服务器的可访问地址
//!
//! 客户端通过信令服务器发现 ws_address 后，直接通过 WebSocket 建立连接。

mod echo_service;
mod generated;

use echo_service::EchoService;
use generated::echo_service_actor::EchoServiceWorkload;

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

    info!("🚀 WS Echo Server 启动");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📡 WebSocket 直连模式");
    info!(
        "🔌 WebSocket 监听端口: {:?}",
        config.websocket_listen_port
    );
    info!(
        "📣 广播地址: {:?}",
        config.websocket_advertised_host
    );
    info!("📡 需要 signaling-server 运行在 ws://localhost:8081");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. 创建 ActorSystem
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🏗️  创建 ActrSystem...");

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
    // 4. 启动 ActrNode（连接信令服务器、注册 ws_address、监听端口）
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("🚀 启动 ActrNode...");
    info!("   (节点将把 WebSocket 地址注册到信令服务器)");

    let actr_ref = match node.start().await {
        Ok(actr) => actr,
        Err(e) => {
            error!("❌ ActrNode 启动失败: {:?}", e);
            error!("💡 提示：请确保 signaling-server 已运行并且 realm 已配置");
            return Err(e.into());
        }
    };

    info!("✅ ActrNode 启动成功！");
    info!("🆔 Server ID: {:?}", actr_ref.actor_id());
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("🎉 WS Echo Server 已完全启动并注册");
    info!("🔌 WebSocket 地址已上报到信令服务器");
    info!("📡 等待客户端通过 WebSocket 连接...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 5. 等待 Ctrl+C 并关闭
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ WS Echo Server 已关闭");

    Ok(())
}
