//! Echo Real Server - 真实的 Actor 服务端
//!
//! 使用 ActorSystem 启动，通过 signaling server 注册

mod echo_service;
mod generated;

use echo_service::EchoService;
use generated::echo_service_actor::EchoServiceWorkload;

use actr_runtime::prelude::*;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("🚀 Echo Real Server 启动");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📝 使用真实的 protobuf 注册流程");
    info!("📡 需要 signaling-server 运行在 ws://localhost:8081");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 1. 创建配置
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    info!("⚙️  创建配置...");

    use actr_protocol::{ActrType, Realm};
    use std::collections::HashMap;

    let config = actr_config::Config {
        package: actr_config::PackageInfo {
            name: "echo-real-server".to_string(),
            actr_type: ActrType {
                manufacturer: "acme".to_string(),
                name: "echo.EchoService".to_string(),
            },
            description: Some("Echo Real Server".to_string()),
            authors: vec![],
            license: Some("Apache-2.0".to_string()),
        },
        exports: vec![],
        dependencies: vec![],
        signaling_url: url::Url::parse("ws://localhost:8081/signaling/ws")?,
        realm: Realm { realm_id: 0 },
        visible_in_discovery: true,
        acl: None,
        mailbox_path: None, // Use in-memory database
        tags: vec!["dev".to_string(), "example".to_string()],
        scripts: HashMap::new(),
    };

    info!("✅ 配置已创建");

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
