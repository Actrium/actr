//! Sender Workload - Sends DataStream chunks

use actr_runtime::prelude::*;
use actr_protocol::{DataStream, ActrId, ActrType, RpcEnvelope};
use actr_framework::{Workload, Context, Dest, MessageDispatcher, Bytes};
use async_trait::async_trait;

pub struct SenderWorkload {
    receiver_id: ActrId,
}

// Empty dispatcher since we don't handle RPC messages
pub struct EmptyDispatcher;

#[async_trait]
impl MessageDispatcher for EmptyDispatcher {
    type Workload = SenderWorkload;

    async fn dispatch<C: Context>(
        _workload: &Self::Workload,
        _envelope: RpcEnvelope,
        _ctx: &C,
    ) -> ActorResult<Bytes> {
        Err(actr_protocol::ProtocolError::Actr(actr_protocol::ActrError::NotImplemented {
            feature: "RPC not implemented for this workload".to_string(),
        }))
    }
}

impl SenderWorkload {
    pub fn new(receiver_id: ActrId) -> Self {
        Self { receiver_id }
    }
}

#[async_trait]
impl Workload for SenderWorkload {
    type Dispatcher = EmptyDispatcher;

    fn actor_type(&self) -> ActrType {
        ActrType {
            manufacturer: "acme".to_string(),
            name: "datastream.Sender".to_string(),
        }
    }

    async fn on_start<C: Context + Send + Sync>(&self, ctx: &C) -> ActorResult<()> {
        tracing::info!("🎬 SenderWorkload starting");
        tracing::info!("🎯 Target receiver: {:?}", self.receiver_id);

        // Wait a bit for WebRTC connection
        tracing::info!("⏳ Waiting 3 seconds for WebRTC connection...");
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        tracing::info!("📤 Starting data stream transmission...");

        let stream_id = "file-transfer".to_string();
        let dest = Dest::Actor(self.receiver_id.clone());

        // Send 10 chunks
        for i in 0..10 {
            let data = format!("Chunk #{}: Hello from DataStream API! This is chunk number {}.", i, i);

            let chunk = DataStream {
                stream_id: stream_id.clone(),
                sequence: i,
                payload: bytes::Bytes::from(data.clone()),
                metadata: vec![],
                timestamp_ms: Some(chrono::Utc::now().timestamp_millis()),
            };

            tracing::info!(
                "📤 Sending chunk #{} (sequence={}, size={} bytes)",
                i,
                chunk.sequence,
                chunk.payload.len()
            );

            match ctx.send_data_stream(&dest, chunk).await {
                Ok(_) => tracing::info!("✅ Chunk #{} sent successfully", i),
                Err(e) => tracing::error!("❌ Failed to send chunk #{}: {:?}", i, e),
            }

            // Sleep between chunks
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        tracing::info!("✅ All chunks sent!");

        Ok(())
    }

    async fn on_stop<C: Context + Send + Sync>(&self, _ctx: &C) -> ActorResult<()> {
        tracing::info!("🛑 SenderWorkload stopping");
        Ok(())
    }
}
