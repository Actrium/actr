//! DataStream Sender Example - 100% Real Implementation
//!
//! Demonstrates sending data streams using:
//! - RPC for control messages (StartTransfer, EndTransfer)
//! - DataStream API for fast path data transmission

mod generated;

use actr_framework::{Context, Dest, MessageDispatcher, Workload};
use actr_protocol::{
    ActorResult, ActrError, ActrType, DataStream, ProtocolError, Realm, RpcEnvelope,
};
use actr_runtime::prelude::*;
use bytes::Bytes;
use generated::file_transfer::*;
use std::collections::HashMap;
use tracing::{error, info};
// use prost::Message as ProstMessage;  // 未使用

// Sender workload that performs file transfer in on_start
struct SenderWorkload;

#[async_trait::async_trait]
impl Workload for SenderWorkload {
    type Dispatcher = DummyDispatcher;

    fn actor_type(&self) -> ActrType {
        ActrType {
            manufacturer: "acme".to_string(),
            name: "generic.GenericClient".to_string(),
        }
    }

    async fn on_start<C: Context>(&self, ctx: &C) -> ActorResult<()> {
        info!("🚀 Sender workload started");

        // TODO: Replace sleep with proper service discovery/ready notification
        // Temporary workaround until discovery API is implemented
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        // Discover receiver
        info!("🔍 Discovering FileTransferService receiver...");

        // Receiver identification via environment variables with sensible defaults
        // Defaults align with the receiver example in this repo
        let receiver_manufacturer =
            std::env::var("ACTR_RECEIVER_MANUFACTURER").unwrap_or_else(|_| "acme".to_string());
        let receiver_type_name = std::env::var("ACTR_RECEIVER_TYPE")
            .unwrap_or_else(|_| "file_transfer.FileTransferService".to_string());
        let receiver_serial: u64 = std::env::var("ACTR_RECEIVER_SERIAL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1000);
        let receiver_realm_id: u32 = std::env::var("ACTR_REALM_ID")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let receiver_type = ActrType {
            manufacturer: receiver_manufacturer,
            name: receiver_type_name,
        };

        let receiver_id = ActrId {
            realm: Realm {
                realm_id: receiver_realm_id,
            },
            serial_number: receiver_serial,
            r#type: receiver_type.clone(),
        };

        info!("✅ Using receiver: {:?}", receiver_id);

        // Prepare file content
        let content = "Hello DataStream! This is a test file content. ".repeat(100);
        let chunk_size = 1024;
        let chunks: Vec<Bytes> = content
            .as_bytes()
            .chunks(chunk_size)
            .map(|chunk| Bytes::copy_from_slice(chunk))
            .collect();

        info!("📤 Starting file transfer:");
        info!("   Filename: test-file.txt");
        info!("   Total size: {} bytes", content.len());
        info!("   Chunk size: {} bytes", chunk_size);
        info!("   Chunk count: {}", chunks.len());

        // Phase 1: StartTransfer RPC (Control Plane)
        info!("📡 Phase 1: Sending StartTransfer RPC...");
        let start_req = StartTransferRequest {
            stream_id: "test-stream-001".to_string(),
            filename: "test-file.txt".to_string(),
            total_size: content.len() as u64,
            chunk_count: chunks.len() as u32,
        };

        let start_resp: StartTransferResponse = ctx
            .call(&Dest::Actor(receiver_id.clone()), start_req)
            .await?;

        if !start_resp.ready {
            error!("❌ Receiver not ready: {}", start_resp.message);
            return Err(ProtocolError::TransportError(format!(
                "Receiver not ready: {}",
                start_resp.message
            )));
        }

        info!("✅ StartTransfer RPC succeeded: {}", start_resp.message);

        // Phase 2: Send DataStream chunks (Data Plane - Fast Path)
        info!("📦 Phase 2: Sending {} DataStream chunks...", chunks.len());

        for (i, chunk) in chunks.iter().enumerate() {
            let data_stream = DataStream {
                stream_id: "test-stream-001".to_string(),
                sequence: i as u64,
                payload: chunk.clone(),
                metadata: vec![],
                timestamp_ms: Some(chrono::Utc::now().timestamp_millis()),
            };

            ctx.send_data_stream(&Dest::Actor(receiver_id.clone()), data_stream)
                .await?;

            let progress = ((i + 1) as f64 / chunks.len() as f64 * 100.0) as u32;
            info!(
                "   Sent chunk #{}/{}: {} bytes ({}%)",
                i + 1,
                chunks.len(),
                chunk.len(),
                progress
            );

            // Small delay to avoid overwhelming receiver
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        info!("✅ All chunks sent successfully");

        // Phase 3: EndTransfer RPC (Control Plane)
        info!("🏁 Phase 3: Sending EndTransfer RPC...");

        let end_req = EndTransferRequest {
            stream_id: "test-stream-001".to_string(),
            success: true,
        };

        let end_resp: EndTransferResponse = ctx.call(&Dest::Actor(receiver_id), end_req).await?;

        info!("✅ EndTransfer RPC succeeded!");
        info!("📊 Transfer Statistics:");
        info!("   Acknowledged: {}", end_resp.acknowledged);
        info!("   Chunks received: {}", end_resp.chunks_received);
        info!("   Bytes received: {}", end_resp.bytes_received);
        info!("🎉 File transfer completed successfully!");

        Ok(())
    }
}

struct DummyDispatcher;

#[async_trait::async_trait]
impl MessageDispatcher for DummyDispatcher {
    type Workload = SenderWorkload;

    async fn dispatch<C: Context>(
        _workload: &Self::Workload,
        envelope: RpcEnvelope,
        _ctx: &C,
    ) -> ActorResult<Bytes> {
        Err(ProtocolError::Actr(ActrError::UnknownRoute {
            route_key: envelope.route_key.to_string(),
        }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("🚀 DataStream Sender starting - 100% Real Implementation");

    // Create config
    let config = actr_config::Config {
        package: actr_config::PackageInfo {
            name: "file-transfer-sender".to_string(),
            actr_type: ActrType {
                manufacturer: "acme".to_string(),
                name: "generic.GenericClient".to_string(),
            },
            description: Some("File transfer sender using DataStream API".to_string()),
            authors: vec![],
            license: Some("Apache-2.0".to_string()),
        },
        exports: vec![],
        dependencies: vec![],
        signaling_url: url::Url::parse("ws://localhost:8081/signaling/ws")?,
        realm: Realm { realm_id: 0 },
        visible_in_discovery: true,
        acl: None,
        mailbox_path: None,
        tags: vec!["dev".to_string(), "example".to_string()],
        scripts: HashMap::new(),
    };

    // Create ActrSystem
    info!("🏗️  Creating ActrSystem...");
    let system = ActrSystem::new(config).await?;
    info!("✅ ActrSystem created");

    // Attach sender workload
    let node = system.attach(SenderWorkload);

    info!("🚀 Starting ActrNode...");
    let actr_ref = node.start().await?;

    info!("✅ ActrNode started! Actor ID: {:?}", actr_ref.actor_id());

    // Wait for transfer to complete
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

    info!("👋 Sender shutting down");
    Ok(())
}
