//! FileTransfer Service Implementation - Receiver

use crate::generated::file_transfer::*;
use crate::generated::file_transfer_service_actor::FileTransferServiceHandler;
use actr_protocol::{ActorResult, DataStream};
use actr_runtime::prelude::*;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

/// FileTransfer 服务实现 - 接收端
///
/// 负责：
/// 1. 处理 StartTransfer RPC - 注册 DataStream 回调
/// 2. 接收 DataStream 数据块
/// 3. 处理 EndTransfer RPC - 确认传输完成
pub struct FileTransferService {
    /// 接收到的数据块计数
    chunks_received: Arc<Mutex<u32>>,
    /// 接收到的总字节数
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
    /// StartTransfer - 启动文件传输
    ///
    /// 1. 注册 DataStream 回调
    /// 2. 返回 ready=true 通知 sender 可以开始发送
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

        // 重置计数器
        *self.chunks_received.lock().await = 0;
        *self.bytes_received.lock().await = 0;

        // 注册 DataStream 回调
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

                // 如果是文本数据，显示预览
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

    /// EndTransfer - 结束文件传输
    ///
    /// 1. 取消注册 DataStream 回调
    /// 2. 返回接收统计信息
    async fn end_transfer<C: Context>(
        &self,
        req: EndTransferRequest,
        ctx: &C,
    ) -> ActorResult<EndTransferResponse> {
        tracing::info!("🏁 EndTransfer RPC received:");
        tracing::info!("   stream_id: {}", req.stream_id);
        tracing::info!("   success: {}", req.success);

        // 取消注册回调
        ctx.unregister_stream(&req.stream_id).await?;
        tracing::info!(
            "✅ DataStream callback unregistered for '{}'",
            req.stream_id
        );

        // 获取统计信息
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
