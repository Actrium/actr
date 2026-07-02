//! DataStreamRegistry - Fast path data stream registry

use actr_protocol::{ActorResult, ActrId, DataStream};
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
pub(crate) type DataStreamCallback =
    Arc<dyn Fn(DataStream, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync>;

/// DataStreamRegistry - Stream chunk callback manager
///
/// # Responsibilities
/// - Receive DataStream from LatencyFirst Lane (stream-format data packets)
/// - Maintain stream_id → callback mapping
/// - Concurrently invoke user-registered data stream callbacks
///
/// # Typical Use Cases
/// - Streaming RPC (peer push streams)
/// - Real-time collaborative editing (multi-user editing sync)
/// - Game state streams (position updates, event streams)
/// - Log streams, sensor data streams, metrics streams
pub(crate) struct DataStreamRegistry {
    /// Concurrent mapping of stream_id → callback function
    callbacks: DashMap<String, DataStreamCallback>,
}

impl Default for DataStreamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DataStreamRegistry {
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
    pub(crate) fn register(&self, stream_id: String, callback: DataStreamCallback) {
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
    pub(crate) async fn dispatch(&self, chunk: DataStream, sender_id: ActrId) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn chunk(stream_id: &str) -> DataStream {
        DataStream {
            stream_id: stream_id.to_string(),
            sequence: 1,
            payload: Default::default(),
            metadata: vec![],
            timestamp_ms: None,
        }
    }

    fn counting_callback() -> (DataStreamCallback, Arc<Mutex<u32>>) {
        let count = Arc::new(Mutex::new(0u32));
        let c = count.clone();
        let cb: DataStreamCallback = Arc::new(move |_chunk, _sender| {
            let c = c.clone();
            Box::pin(async move {
                *c.lock().unwrap() += 1;
                Ok(())
            })
        });
        (cb, count)
    }

    #[test]
    fn register_and_default() {
        let reg = DataStreamRegistry::default();
        assert_eq!(reg.callbacks.len(), 0);
        let (cb, _) = counting_callback();
        reg.register("s1".into(), cb);
        assert_eq!(reg.callbacks.len(), 1);
    }

    #[test]
    fn unregister_removes_stream() {
        let reg = DataStreamRegistry::new();
        let (cb, _) = counting_callback();
        reg.register("s1".into(), cb);
        reg.unregister("s1");
        assert_eq!(reg.callbacks.len(), 0);
        // Unknown id is a no-op.
        reg.unregister("never");
        assert_eq!(reg.callbacks.len(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dispatch_invokes_registered_callback() {
        let reg = DataStreamRegistry::new();
        let (cb, count) = counting_callback();
        reg.register("s1".into(), cb);

        reg.dispatch(chunk("s1"), ActrId::default()).await;
        for _ in 0..50 {
            if *count.lock().unwrap() == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(*count.lock().unwrap(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dispatch_unknown_stream_is_noop() {
        let reg = DataStreamRegistry::new();
        reg.dispatch(chunk("missing"), ActrId::default()).await;
        assert_eq!(reg.callbacks.len(), 0);
    }
}
