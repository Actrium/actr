//! DataChunkRegistry - Fast path data stream registry

use actr_protocol::{ActorResult, ActrId, DataChunk};
use dashmap::DashMap;
use futures_util::future::BoxFuture;
use std::sync::Arc;

/// Stream chunk callback type
///
/// # Design Rationale
/// Fast path is stream-based push, not RPC, so it doesn't need full Context:
/// - Only passes sender ActrId (to know where data comes from)
/// - Doesn't pass Context (avoids confusing RPC and Stream semantics)
/// - If reverse signaling needed, user should send via OutboundGate
pub(crate) type DataChunkCallback =
    Arc<dyn Fn(DataChunk, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync>;

/// DataChunkRegistry - Stream chunk callback manager
///
/// # Responsibilities
/// - Receive DataChunk from LatencyFirst Lane (stream-format data packets)
/// - Maintain stream_id → callback mapping
/// - Concurrently invoke user-registered data stream callbacks
///
/// # Typical Use Cases
/// - Streaming RPC (peer push streams)
/// - Real-time collaborative editing (multi-user editing sync)
/// - Game state streams (position updates, event streams)
/// - Log streams, sensor data streams, metrics streams
pub(crate) struct DataChunkRegistry {
    /// Concurrent mapping of stream_id → callback function
    callbacks: DashMap<String, DataChunkCallback>,
}

impl Default for DataChunkRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DataChunkRegistry {
    pub(crate) fn new() -> Self {
        Self {
            callbacks: DashMap::new(),
        }
    }

    /// Register stream callback
    ///
    /// # Arguments
    /// - `stream_id`: stream identifier (must be globally unique)
    /// - `callback`: data stream handler callback
    pub(crate) fn register(&self, stream_id: String, callback: DataChunkCallback) {
        self.callbacks.insert(stream_id.clone(), callback);
        tracing::info!("📡 Registered data stream handler: {}", stream_id);
    }

    /// Unregister stream callback
    ///
    /// # Arguments
    /// - `stream_id`: stream identifier to unregister
    pub(crate) fn unregister(&self, stream_id: &str) {
        self.callbacks.remove(stream_id);
        tracing::info!("🚫 Unregistered data stream handler: {}", stream_id);
    }

    /// Remove every registered stream callback.
    ///
    /// Node teardown uses this to break callback ownership chains before the
    /// transport and runtime contexts are released. `DashMap::clear` is safe
    /// to race with dispatch: a callback already cloned by [`Self::dispatch`]
    /// is allowed to finish while the registered entries are removed.
    pub(crate) fn clear(&self) {
        let count = self.callbacks.len();
        self.callbacks.clear();
        if count > 0 {
            tracing::debug!(count, "Cleared data stream callbacks");
        }
    }

    /// Dispatch data stream to callback (concurrent execution)
    ///
    /// # Arguments
    /// - `chunk`: data stream
    /// - `sender_id`: sender ActrId
    ///
    /// # Performance
    /// - Direct callback invocation, no queueing overhead
    /// - Latency: ~10μs
    /// - Concurrent execution, doesn't block other streams
    pub(crate) async fn dispatch(&self, chunk: DataChunk, sender_id: ActrId) {
        let start = std::time::Instant::now();

        if let Some(callback) = self.callbacks.get(&chunk.stream_id) {
            let callback = callback.clone();
            tokio::spawn(async move {
                if let Err(e) = callback(chunk, sender_id).await {
                    tracing::error!("❌ Stream chunk callback error: {:?}", e);
                }
            });

            tracing::debug!("🚀 Dispatched data stream in {:?}", start.elapsed());
        } else {
            tracing::warn!("⚠️ No callback registered for stream: {}", chunk.stream_id);
        }
    }

    /// Invoke one callback inline so ownership and host-bridge behaviour can
    /// be asserted deterministically without observing a detached task.
    #[cfg(test)]
    pub(crate) async fn invoke_for_test(
        &self,
        chunk: DataChunk,
        sender_id: ActrId,
    ) -> Option<ActorResult<()>> {
        let callback = self
            .callbacks
            .get(&chunk.stream_id)
            .map(|entry| entry.value().clone())?;
        Some(callback(chunk, sender_id).await)
    }
}

#[cfg(test)]
#[path = "data_chunk_registry_tests.rs"]
mod tests;
