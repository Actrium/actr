use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::{error, info};

mod client_workload;
mod generated;

use actr_protocol::ActrType;
use actr_runtime::prelude::*;
use client_workload::ClientWorkload;

#[tokio::main]
async fn main() -> Result<()> {
    // 加载配置文件
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // 初始化可观测性（日志/链路追踪）
    let _obs_guard = actr_runtime::init_observability(&config.observability)?;

    info!("🚀 Echo Client App - 并发测试模式");

    let system = ActrSystem::new(config).await?;
    info!("✅ ActrSystem 创建成功");

    // 准备 workload 并配置服务器目标
    let workload = ClientWorkload::new();

    info!("🌐 通过信令服务器发现 echo server...");
    let target_type = ActrType {
        manufacturer: "acme".to_string(),
        name: "EchoService".to_string(),
        version: "v1".to_string(),
    };

    let node = system.attach(workload.clone());

    info!("🚀 启动 ActrNode...");
    let actr_ref = node.start().await?;
    let mut candidates = actr_ref
        .discover_route_candidates(&target_type, 1)
        .await
        .context("从信令服务器查询 echo 候选节点失败")?;

    let server_id = candidates
        .pop()
        .ok_or_else(|| anyhow::anyhow!("没有可用的 echo server 实例"))?;

    info!("🎯 目标服务器: {:?}", server_id);
    workload.set_server_id(server_id).await;
    let local_id = actr_ref.actor_id().clone();
    info!("✅ ActrNode 启动成功，ID: {:?}", local_id);

    // 构造包含客户端 ID 的请求消息
    use generated::echo::{EchoRequest, EchoResponse};

    let client_id_str = format!("{:?}", local_id);
    let message = format!("Client[{}] says hello!", client_id_str);

    let request = EchoRequest {
        message: message.clone(),
    };

    info!("📤 发送 echo 请求: {}", request.message);
    println!("\n🔹 Client ID: {}", client_id_str);
    println!("🔹 发送消息: {}", message);

    match actr_ref.call(request).await {
        Ok(response) => {
            let response: EchoResponse = response;
            info!("📥 收到 echo 响应: {}", response.reply);
            info!("⏰ 时间戳: {}", response.timestamp);

            // 验证响应中是否包含客户端 ID
            if response.reply.contains(&client_id_str) {
                println!("\n✅ 成功！响应匹配当前客户端");
                println!("  客户端 ID:  {}", client_id_str);
                println!("  发送消息:   {}", message);
                println!("  服务器响应: {}", response.reply);
                println!("  时间戳:     {}", response.timestamp);
                println!("  ✓ 验证通过: 响应包含客户端 ID");
            } else {
                println!("\n⚠️  警告！响应不匹配");
                println!("  预期包含:   {}", client_id_str);
                println!("  实际响应:   {}", response.reply);
            }
        }
        Err(e) => {
            error!("❌ 调用 echo 服务失败: {:?}", e);
            println!("\n❌ 错误: {}", e);
        }
    }

    info!("👋 应用程序关闭");
    Ok(())
}
