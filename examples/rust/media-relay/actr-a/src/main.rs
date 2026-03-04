//! Actr A (Relay) - Real Shell client implementation
//!
//! Sends media frames to Actr B via real WebRTC P2P RPC calls

mod generated;
mod relay_client_workload;

use generated::media_relay::*;
use media_relay_common::{MediaSource, TestPatternSource};
use relay_client_workload::RelayClientWorkload;

use actr_protocol::ActrType;
use actr_runtime::prelude::*;
use anyhow::{Context, anyhow};
use std::path::PathBuf;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Load configuration from Actr.toml
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // Initialize observability (logging/tracing) using config
    let _obs_guard = actr_runtime::init_observability(&config.observability)?;

    info!("🚀 Actr A (Relay/Shell Client) 启动");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📝 使用真实的 ActrRef Shell API");
    info!("📡 将通过 WebRTC P2P 发送媒体帧到 Actr B");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // 2. Create ActrSystem
    info!("🏗️  创建 ActrSystem...");

    let system = ActrSystem::new(config).await?;

    info!("✅ ActrSystem 创建成功");

    // 3. Create RelayClientWorkload and set target server
    info!("📦 创建 RelayClientWorkload...");

    let workload = RelayClientWorkload::new();
    let node = system.attach(workload.clone());

    info!("✅ RelayClientWorkload 已附加");

    // 4. Start ActrNode
    info!("🚀 启动 ActrNode...");

    let actr_ref = node.start().await?;

    info!("✅ ActrNode 启动成功！");
    info!("📍 本地 Actor ID: {:?}", actr_ref.actor_id());

    // 4.1 Discover remote actr-b
    let target_type = ActrType {
        manufacturer: "actr-example".to_string(),
        name: "media_relay.RelayService".to_string(),
    };
    info!("🌐 通过 signaling server 发现 Actr B...");
    let mut candidates = actr_ref
        .discover_route_candidates(&target_type, 1)
        .await
        .context("Failed to query signaling server for Actr B")?;

    let server_id = candidates
        .pop()
        .ok_or_else(|| anyhow!("No Actr B instances available from signaling server"))?;
    info!("🎯 目标 Actor: {:?}", server_id);
    workload.set_server_id(server_id).await;

    // 5. Generate and send frames
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📤 开始发送媒体帧 via ActrRef Shell API...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let mut video_source = TestPatternSource::new(30);

    for i in 0..10 {
        if let Some(sample) = video_source.next_sample() {
            info!(
                "📹 生成帧 #{}: {} bytes, ts={}, codec={}",
                i,
                sample.data.len(),
                sample.timestamp,
                sample.codec
            );

            let frame = MediaFrame {
                data: sample.data.to_vec(),
                timestamp: sample.timestamp,
                codec: sample.codec.clone(),
                frame_number: i as u32,
            };

            let request = RelayFrameRequest { frame: Some(frame) };

            // Call local workload via ActrRef, which forwards to remote actr-b
            match actr_ref.call(request).await {
                Ok(response) => {
                    info!(
                        "   ✅ 帧 #{} 已发送，服务器确认: success={}, received_at={}",
                        i, response.success, response.received_at
                    );
                }
                Err(e) => {
                    info!("   ❌ 帧 #{} 发送失败: {:?}", i, e);
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(33)).await;
        }
    }

    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("✅ Actr A 完成发送 10 帧");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Keep running to allow shutdown
    info!("Press Ctrl+C to shutdown...");
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ Actr A 已关闭");

    Ok(())
}
