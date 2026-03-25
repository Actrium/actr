//! FileTransfer Service Implementation - Receiver

use crate::generated::file_transfer::*;
use crate::generated::file_transfer_actor::FileTransferServiceHandler;
use actr_protocol::{ActorResult, DataStream};
use actr_hyper::prelude::*;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

/// FileTransfer service[...] - receive[...]
///
/// [...]：
/// 1. [...] StartTransfer RPC - register DataStream [...]
/// 2. receive DataStream data[...]
/// 3. [...] EndTransfer RPC - [...]
pub struct FileTransferService {
    /// receive[...]data[...]
    chunks_received: Arc<Mutex<u32>>,
    /// receive[...]
    bytes_received: Arc<Mutex<u64>>,
}

impl FileTransferService {
    pub fn new() -> Self {
        Self {
            chunks_received: Arc::new(Mutex::new(0)),
            bytes_received: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait]
impl FileTransferServiceHandler for FileTransferService {
    /// StartTransfer - start[...]
    ///
    /// 1. register DataStream [...]
    /// 2. [...] ready=true [...] sender [...]send
    async fn start_transfer<C: Context>(
        &self,
        req: StartTransferRequest,
        ctx: &C,
    ) -> ActorResult<StartTransferResponse> {
        tracing::info!("📥 StartTransfer RPC received:");
        tracing::info!("   stream_id: {}", req.stream_id);
        tracing::info!("   filename: {}", req.filename);
        tracing::info!("   total_size: {} bytes", req.total_size);
        tracing::info!("   chunk_count: {}", req.chunk_count);

        // [...]
        *self.chunks_received.lock().await = 0;
        *self.bytes_received.lock().await = 0;

        // register DataStream [...]
        let stream_id = req.stream_id.clone();
        let chunks_counter = self.chunks_received.clone();
        let bytes_counter = self.bytes_received.clone();
        let total_size = req.total_size;
        let chunk_count = req.chunk_count;

        ctx.register_stream(stream_id.clone(), move |data_stream: DataStream, sender_id: ActrId| {
            let chunks_counter = chunks_counter.clone();
            let bytes_counter = bytes_counter.clone();

            Box::pin(async move {
                let mut chunks = chunks_counter.lock().await;
                let mut bytes = bytes_counter.lock().await;

                *chunks += 1;
                *bytes += data_stream.payload.len() as u64;

                let progress = (*bytes as f64 / total_size as f64 * 100.0) as u32;

                tracing::info!(
                    "📦 Received chunk #{}/{} from {:?}: sequence={}, size={} bytes, progress={}%, total_bytes={}",
                    *chunks,
                    chunk_count,
                    sender_id,
                    data_stream.sequence,
                    data_stream.payload.len(),
                    progress,
                    *bytes
                );

                // [...]data，[...]
                if let Ok(text) = String::from_utf8(data_stream.payload.to_vec()) {
                    let preview = &text[..text.len().min(80)];
                    tracing::debug!("   Content preview: {}", preview);
                }

                Ok(())
            })
        }).await?;

        tracing::info!("✅ DataStream callback registered for '{}'", stream_id);
        tracing::info!(
            "   Ready to receive {} chunks ({} bytes)",
            chunk_count,
            total_size
        );

        Ok(StartTransferResponse {
            ready: true,
            message: format!("Ready to receive {} chunks", chunk_count),
        })
    }

    /// EndTransfer - [...]
    ///
    /// 1. [...]register DataStream [...]
    /// 2. [...]receive[...]
    async fn end_transfer<C: Context>(
        &self,
        req: EndTransferRequest,
        ctx: &C,
    ) -> ActorResult<EndTransferResponse> {
        tracing::info!("🏁 EndTransfer RPC received:");
        tracing::info!("   stream_id: {}", req.stream_id);
        tracing::info!("   success: {}", req.success);

        // [...]register[...]
        ctx.unregister_stream(&req.stream_id).await?;
        tracing::info!(
            "✅ DataStream callback unregistered for '{}'",
            req.stream_id
        );

        // [...]
        let chunks = *self.chunks_received.lock().await;
        let bytes = *self.bytes_received.lock().await;

        tracing::info!("📊 Transfer complete:");
        tracing::info!("   Chunks received: {}", chunks);
        tracing::info!("   Bytes received: {}", bytes);

        Ok(EndTransferResponse {
            acknowledged: true,
            bytes_received: bytes,
            chunks_received: chunks,
        })
    }
}
