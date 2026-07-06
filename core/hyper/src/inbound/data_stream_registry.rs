//! DataStreamRegistry - Fast path data stream registry

use actr_protocol::{ActorResult, ActrId, DataStream, PayloadType};
use dashmap::{DashMap, mapref::entry::Entry};
use futures_util::{FutureExt, future::BoxFuture};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Stream chunk callback type
///
/// # Design Rationale
/// Fast path is stream-based push, not RPC, so it doesn't need full Context:
/// - Only passes sender ActrId (to know where data comes from)
/// - Doesn't pass Context (avoids confusing RPC and Stream semantics)
/// - If reverse signaling needed, user should send via OutboundGate
pub(crate) type DataStreamCallback =
    Arc<dyn Fn(DataStream, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct StreamKey {
    sender_id: ActrId,
    stream_id: String,
}

struct QueuedDataStream {
    chunk: DataStream,
    sender_id: ActrId,
    callback: DataStreamCallback,
}

struct StreamWorker {
    tx: mpsc::UnboundedSender<QueuedDataStream>,
}

/// DataStreamRegistry - Stream chunk callback manager
///
/// # Responsibilities
/// - Receive DataStream from LatencyFirst Lane (stream-format data packets)
/// - Maintain stream_id → callback mapping
/// - Preserve FIFO callback processing per `(sender_id, stream_id)` for reliable streams
/// - Keep latency-first streams concurrent
///
/// # Typical Use Cases
/// - Streaming RPC (peer push streams)
/// - Real-time collaborative editing (multi-user editing sync)
/// - Game state streams (position updates, event streams)
/// - Log streams, sensor data streams, metrics streams
pub(crate) struct DataStreamRegistry {
    /// Concurrent mapping of stream_id → callback function
    callbacks: DashMap<String, DataStreamCallback>,
    workers: DashMap<StreamKey, StreamWorker>,
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
            workers: DashMap::new(),
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
        self.workers.retain(|key, _| key.stream_id != stream_id);
        tracing::info!("🚫 Unregistered data stream handler: {}", stream_id);
    }

    fn get_or_spawn_worker(&self, key: StreamKey) -> mpsc::UnboundedSender<QueuedDataStream> {
        if let Some(worker) = self.workers.get(&key) {
            return worker.tx.clone();
        }

        let (tx, rx) = mpsc::unbounded_channel();
        match self.workers.entry(key.clone()) {
            Entry::Occupied(entry) => entry.get().tx.clone(),
            Entry::Vacant(entry) => {
                tokio::spawn(Self::run_worker(key, rx));
                entry.insert(StreamWorker { tx: tx.clone() });
                tx
            }
        }
    }

    async fn run_callback(item: QueuedDataStream) {
        let stream_id = item.chunk.stream_id.clone();
        let sender_id = item.sender_id.clone();
        let result = std::panic::AssertUnwindSafe((item.callback)(item.chunk, item.sender_id))
            .catch_unwind()
            .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::error!("❌ Stream chunk callback error: {:?}", e),
            Err(_) => tracing::error!(
                stream_id = %stream_id,
                sender_id = ?sender_id,
                "❌ Stream chunk callback panicked"
            ),
        }
    }

    async fn run_worker(key: StreamKey, mut rx: mpsc::UnboundedReceiver<QueuedDataStream>) {
        while let Some(item) = rx.recv().await {
            Self::run_callback(item).await;
        }

        tracing::debug!(
            stream_id = %key.stream_id,
            sender_id = ?key.sender_id,
            "DataStream worker stopped"
        );
    }

    /// Dispatch data stream to callback.
    ///
    /// # Arguments
    /// - `chunk`: data stream
    /// - `sender_id`: sender ActrId
    ///
    /// # Performance
    /// - `StreamReliable`: same `(sender_id, stream_id)` is processed FIFO by one worker.
    /// - `StreamLatencyFirst`: chunks are spawned independently to avoid head-of-line blocking.
    /// - Different reliable streams can still run concurrently.
    pub(crate) async fn dispatch(
        &self,
        chunk: DataStream,
        sender_id: ActrId,
        payload_type: PayloadType,
    ) {
        let start = std::time::Instant::now();

        if let Some(callback) = self.callbacks.get(&chunk.stream_id) {
            let callback = callback.clone();
            let key = StreamKey {
                sender_id: sender_id.clone(),
                stream_id: chunk.stream_id.clone(),
            };
            let item = QueuedDataStream {
                chunk,
                sender_id,
                callback,
            };

            match payload_type {
                PayloadType::StreamReliable => {
                    let tx = self.get_or_spawn_worker(key.clone());
                    if let Err(err) = tx.send(item) {
                        tracing::warn!(
                            stream_id = %key.stream_id,
                            sender_id = ?key.sender_id,
                            "DataStream worker channel closed; recreating worker"
                        );
                        self.workers.remove(&key);
                        let retry_tx = self.get_or_spawn_worker(key);
                        if retry_tx.send(err.0).is_err() {
                            tracing::error!(
                                "❌ Failed to enqueue DataStream chunk after worker restart"
                            );
                        }
                    }
                }
                PayloadType::StreamLatencyFirst => {
                    tokio::spawn(Self::run_callback(item));
                }
                other => {
                    tracing::warn!("⚠️ Unsupported data stream payload type: {:?}", other);
                }
            }

            tracing::debug!(
                "🚀 Dispatched data stream ({:?}) in {:?}",
                payload_type,
                start.elapsed()
            );
        } else {
            tracing::warn!("⚠️ No callback registered for stream: {}", chunk.stream_id);
        }
    }
}

#[cfg(test)]
#[path = "data_stream_registry_tests.rs"]
mod tests;
