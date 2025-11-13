//! Actr A (Relay) - Real Shell client implementation
//!
//! Sends media frames to Actr B via real WebRTC P2P RPC calls

mod generated;
mod relay_client_workload;

use generated::media_relay::*;
use media_relay_common::{MediaSource, TestPatternSource};
use relay_client_workload::RelayClientWorkload;

use actr_protocol::{ActrId, ActrType, Realm};
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

    info!("🚀 Actr A (Relay/Shell Client) 启动");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📝 使用真实的 ActrRef Shell API");
    info!("📡 将通过 WebRTC P2P 发送媒体帧到 Actr B");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // 1. Load configuration from Actr.toml
    info!("⚙️  加载配置...");

    let config = actr_config::ConfigParser::from_file("Actr.toml")?;

    info!("✅ 配置已加载");

    // 2. Create ActrSystem
    info!("🏗️  创建 ActrSystem...");

    let system = ActrSystem::new(config).await?;

    info!("✅ ActrSystem 创建成功");

    // 3. Create RelayClientWorkload and set target server
    info!("📦 创建 RelayClientWorkload...");

    let workload = RelayClientWorkload::new();

    // Set target actr-b server ID
    // Note: In production, you would discover this via service discovery
    // Using realm=0 and serial=1000 as assigned by signaling server
    let server_id = ActrId {
        realm: Realm { realm_id: 0 },
        serial_number: 1000,
        r#type: ActrType {
            manufacturer: "actr-example".to_string(),
            name: "media_relay.RelayService".to_string(),
        },
    };
    info!("🎯 目标 Actor: {:?}", server_id);
    workload.set_server_id(server_id).await;

    let node = system.attach(workload);

    info!("✅ RelayClientWorkload 已附加");

    // 4. Start ActrNode
    info!("🚀 启动 ActrNode...");

    let actr_ref = node.start().await?;

    info!("✅ ActrNode 启动成功！");
    info!("📍 本地 Actor ID: {:?}", actr_ref.actor_id());

    // Wait a bit for actr-b to be ready
    info!("⏳ 等待 3 秒，确保 Actr B 已启动...");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

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
