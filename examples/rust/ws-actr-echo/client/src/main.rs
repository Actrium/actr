use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::info;

mod app_side;
mod client_workload;
mod generated;

use actr_protocol::ActrType;
use actr_runtime::prelude::*;
use app_side::AppSide;
use client_workload::ClientWorkload;

#[tokio::main]
async fn main() -> Result<()> {
    // 加载配置
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // 初始化可观测性
    let _obs_guard = actr_runtime::init_observability(&config.observability)?;

    info!("🚀 WS Echo Client App");
    info!("   通过信令服务器发现服务端 WebSocket 地址后直连");

    let system = ActrSystem::new(config).await?;
    info!("✅ ActrSystem 创建成功");

    // 准备 workload
    let workload = ClientWorkload::new();

    // 通过信令服务器发现 WsEchoService
    let target_type = ActrType {
        manufacturer: "acme".to_string(),
        name: "WsEchoService".to_string(),
        version: "1.0.0".to_string(),
    };

    let node = system.attach(workload.clone());

    info!("🚀 启动 ActrNode...");
    let actr_ref = node.start().await?;

    info!("🌐 通过信令服务器发现 WsEchoService...");
    let mut candidates = actr_ref
        .discover_route_candidates(&target_type, 1)
        .await
        .context("无法从信令服务器发现 WsEchoService 实例")?;

    let server_id = candidates
        .pop()
        .ok_or_else(|| anyhow::anyhow!("未找到可用的 WsEchoService 实例"))?;

    info!("🎯 目标服务器: {:?}", server_id);
    info!("🔌 已通过信令发现 WebSocket 地址，后续通信将使用 WebSocket 直连");
    workload.set_server_id(server_id).await;

    let local_id = actr_ref.actor_id().clone();
    info!("✅ ActrNode 启动成功，ID: {:?}", local_id);

    // 运行交互式应用
    let app_side = AppSide {
        actr_ref: actr_ref.clone(),
    };
    app_side.run().await;

    info!("👋 应用已退出");
    Ok(())
}
