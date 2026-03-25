//! Actr A (Relay) - Real Shell client implementation
//!
//! Sends media frames to Actr B via real WebRTC P2P RPC calls

mod generated;
mod relay_client_workload;

use generated::media_relay::*;
use media_relay_common::{MediaSource, TestPatternSource};
use relay_client_workload::RelayClientWorkload;

use actr_protocol::ActrType;
use actr_hyper::prelude::*;
use anyhow::{Context, anyhow};
use std::path::PathBuf;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Load configuration from actr.toml
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // Initialize observability (logging/tracing) using config
    let _obs_guard = actr_hyper::init_observability(&config.observability)?;

    info!("🚀 Actr A (Relay/Shell Client) start");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📝 using/usereal ActrRef Shell API");
    info!("📡 willvia/through WebRTC P2P send[...] Actr B");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // 2. Create RelayClientWorkload and set target server
    info!("📦 create RelayClientWorkload...");

    let workload = RelayClientWorkload::new();
    let node = unimplemented!(
        "source-defined workload examples were removed; migrate this example to a package-backed host"
    );

    info!("✅ ActrNode createsuccess");

    // 3. Start ActrNode
    info!("🚀 start ActrNode...");

    let actr_ref = node.start().await?;

    info!("✅ ActrNode startsuccess！");
    info!("📍 [...] Actor ID: {:?}", actr_ref.actor_id());

    // 3.1 Discover remote actr-b
    let target_type = ActrType {
        manufacturer: "actr-example".to_string(),
        name: "media_relay.RelayService".to_string(),
        version: "1.0.0".to_string(),
    };
    info!("🌐 via/through signaling server discover Actr B...");
    let mut candidates = actr_ref
        .discover_route_candidates(&target_type, 1)
        .await
        .context("Failed to query signaling server for Actr B")?;

    let server_id = candidates
        .pop()
        .ok_or_else(|| anyhow!("No Actr B instances available from signaling server"))?;
    info!("🎯 [...] Actor: {:?}", server_id);
    workload.set_server_id(server_id).await;

    // 5. Generate and send frames
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📤 [...]send[...] via ActrRef Shell API...");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let mut video_source = TestPatternSource::new(30);

    for i in 0..10 {
        if let Some(sample) = video_source.next_sample() {
            info!(
                "📹 [...] #{}: {} bytes, ts={}, codec={}",
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
                        "   ✅ [...] #{} alreadysend，service[...]: success={}, received_at={}",
                        i, response.success, response.received_at
                    );
                }
                Err(e) => {
                    info!("   ❌ [...] #{} sendfailed: {:?}", i, e);
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(33)).await;
        }
    }

    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("✅ Actr A [...]send 10 [...]");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Keep running to allow shutdown
    info!("Press Ctrl+C to shutdown...");
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    info!("✅ Actr A closed");

    Ok(())
}
