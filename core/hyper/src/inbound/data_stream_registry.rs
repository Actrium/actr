//! DataStreamRegistry - Fast path data stream registry
//!
//! # Dispatch semantics (issue #285 minimal closure)
//!
//! Each `stream_id` gets its own lazily-spawned worker task fed by a bounded
//! mpsc queue. This yields:
//!
//! - **Same stream = FIFO run-to-completion**: chunks of one stream are
//!   delivered to the callback strictly in arrival order, one at a time. The
//!   previous chunk's callback future fully resolves before the next starts.
//! - **Different streams = independent**: each stream has its own worker, so a
//!   slow callback on stream A never blocks stream B.
//! - **Reliable overflow = backpressure**: for `StreamReliable`, a full queue
//!   makes `dispatch` `await` the bounded send. Because the WebRTC gate drives a
//!   single receive loop, this stalls reading (SCTP stop-read backpressure) —
//!   it only engages when the app falls behind by a full queue depth. The
//!   callback still runs on its own worker task; we never inline-await it.
//! - **LatencyFirst overflow = drop**: for `StreamLatencyFirst`, a full queue
//!   drops the newest chunk and bumps `dropped_count`. The wire transport is
//!   already partially-reliable (maxRetransmits), so the app must not rely on
//!   every chunk arriving.
//! - **panic isolation**: a callback panic is caught, counted
//!   (`panic_count`), logged, and the worker continues with the next chunk. A
//!   sibling stream or the receive path is never taken down.
//! - **unregister = drain**: removing a stream drops the stored sender; the
//!   worker drains already-queued chunks then exits (already-accepted chunks are
//!   still delivered, matching reliable "accepted == delivered").
//! - **shutdown = cancel**: `shutdown()` cancels the shared token so workers
//!   drop queued chunks, let any in-flight callback finish, then exit; all
//!   worker handles are joined (with a bounded timeout, else aborted). No
//!   orphan tasks in either path.
//!
//! A worker is never resurrected after its channel closes: once unregistered or
//! shut down, a stream stays gone until explicitly re-registered.

use actr_protocol::{ActorResult, ActrId, DataStream, PayloadType};
use dashmap::DashMap;
use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Default per-stream bounded queue depth.
///
/// Compile-time constant for v1; a configurable buffer model is deferred to the
/// concurrency-model RFC. Tests use [`DataStreamRegistry::with_capacity`].
const DEFAULT_STREAM_QUEUE_DEPTH: usize = 64;

/// Total budget for joining worker tasks during [`DataStreamRegistry::shutdown`].
const SHUTDOWN_JOIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Stream chunk callback type
///
/// # Design Rationale
/// Fast path is stream-based push, not RPC, so it doesn't need full Context:
/// - Only passes sender ActrId (to know where data comes from)
/// - Doesn't pass Context (avoids confusing RPC and Stream semantics)
/// - If reverse signaling needed, user should send via OutboundGate
pub(crate) type DataStreamCallback =
    Arc<dyn Fn(DataStream, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync>;

/// A chunk queued for a stream's worker, carrying the sender identity.
struct QueuedChunk {
    chunk: DataStream,
    sender_id: ActrId,
}

/// Per-stream serial executor: a bounded queue plus the worker task draining it.
struct StreamWorker {
    /// Bounded sender into the worker's queue.
    tx: mpsc::Sender<QueuedChunk>,
    /// Worker task handle (joined on shutdown, detached-to-drain on unregister).
    handle: JoinHandle<()>,
}

/// DataStreamRegistry - Stream chunk callback manager
///
/// # Responsibilities
/// - Receive DataStream from Stream lanes (stream-format data packets)
/// - Maintain stream_id → callback mapping
/// - Serialize callbacks per stream via a bounded per-stream worker (see module docs)
///
/// # Typical Use Cases
/// - Streaming RPC (peer push streams)
/// - Real-time collaborative editing (multi-user editing sync)
/// - Game state streams (position updates, event streams)
/// - Log streams, sensor data streams, metrics streams
pub(crate) struct DataStreamRegistry {
    /// Concurrent mapping of stream_id → callback function.
    callbacks: DashMap<String, DataStreamCallback>,
    /// Concurrent mapping of stream_id → serial worker.
    workers: DashMap<String, StreamWorker>,
    /// Cancellation token shared with every worker; cancelling it drains all.
    shutdown: CancellationToken,
    /// Bounded queue depth applied to newly-spawned workers.
    queue_depth: usize,
    /// Count of callback panics isolated by workers (observability hook).
    panic_count: Arc<AtomicU64>,
    /// Count of chunks dropped due to a full LatencyFirst queue (observability hook).
    dropped_count: Arc<AtomicU64>,
}

impl Default for DataStreamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DataStreamRegistry {
    /// Create a registry with its own root cancellation token.
    pub(crate) fn new() -> Self {
        Self::build(DEFAULT_STREAM_QUEUE_DEPTH, CancellationToken::new())
    }

    /// Create a registry whose worker lifecycle is tied to `shutdown`.
    ///
    /// The node passes a child of its own shutdown token so a node-wide
    /// shutdown drains all stream workers.
    pub(crate) fn with_shutdown(shutdown: CancellationToken) -> Self {
        Self::build(DEFAULT_STREAM_QUEUE_DEPTH, shutdown)
    }

    /// Test constructor with an explicit (typically tiny) queue depth so
    /// overflow / backpressure paths can be exercised deterministically.
    #[cfg(test)]
    pub(crate) fn with_capacity(queue_depth: usize) -> Self {
        Self::build(queue_depth.max(1), CancellationToken::new())
    }

    fn build(queue_depth: usize, shutdown: CancellationToken) -> Self {
        Self {
            callbacks: DashMap::new(),
            workers: DashMap::new(),
            shutdown,
            queue_depth,
            panic_count: Arc::new(AtomicU64::new(0)),
            dropped_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Number of callback panics isolated so far (observability / metric hook).
    #[allow(dead_code)]
    pub(crate) fn panic_count(&self) -> u64 {
        self.panic_count.load(Ordering::Relaxed)
    }

    /// Number of chunks dropped by a full LatencyFirst queue (observability / metric hook).
    #[allow(dead_code)]
    pub(crate) fn dropped_count(&self) -> u64 {
        self.dropped_count.load(Ordering::Relaxed)
    }

    /// Test-only handle to the shared shutdown token so tests can observe
    /// cancellation deterministically without sleeping.
    #[cfg(test)]
    pub(crate) fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
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

    /// Unregister stream callback (drain semantics)
    ///
    /// Removes the callback and drops the stored worker sender. The worker
    /// drains any already-queued chunks and then exits on its own; it is never
    /// resurrected.
    pub(crate) fn unregister(&self, stream_id: &str) {
        self.callbacks.remove(stream_id);
        // Dropping the StreamWorker drops its sender -> the worker sees the
        // channel close after draining and exits. The JoinHandle is detached
        // (dropped), never aborted, so in-flight and queued chunks still run.
        self.workers.remove(stream_id);
        tracing::info!("🚫 Unregistered data stream handler: {}", stream_id);
    }

    /// Dispatch a data stream chunk to its per-stream serial worker.
    ///
    /// # Arguments
    /// - `chunk`: data stream chunk
    /// - `sender_id`: sender ActrId
    /// - `payload_type`: transport class, selecting the overflow policy
    ///   (`StreamReliable` = backpressure, `StreamLatencyFirst` = drop-newest)
    ///
    /// Same-stream chunks are delivered in arrival order, run-to-completion.
    pub(crate) async fn dispatch(
        &self,
        chunk: DataStream,
        sender_id: ActrId,
        payload_type: PayloadType,
    ) {
        let stream_id = chunk.stream_id.clone();

        let callback = match self.callbacks.get(&stream_id) {
            Some(cb) => cb.clone(),
            None => {
                tracing::warn!("⚠️ No callback registered for stream: {}", stream_id);
                return;
            }
        };

        let tx = self.worker_tx(&stream_id, callback);
        let queued = QueuedChunk { chunk, sender_id };

        match payload_type {
            PayloadType::StreamLatencyFirst => match tx.try_send(queued) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    let dropped = self.dropped_count.fetch_add(1, Ordering::Relaxed) + 1;
                    tracing::warn!(
                        stream_id = %stream_id,
                        dropped_total = dropped,
                        "⚠️ LatencyFirst queue full; dropping chunk"
                    );
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!(
                        stream_id = %stream_id,
                        "stream worker closed (unregistered/shutdown); dropping chunk"
                    );
                }
            },
            // StreamReliable (and any non-stream misroute defaults to reliable):
            // block on a full queue to apply stop-read backpressure.
            _ => match tx.try_send(queued) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(queued)) => {
                    tracing::warn!(
                        stream_id = %stream_id,
                        "backpressure engaged, inbound stalled"
                    );
                    if tx.send(queued).await.is_err() {
                        tracing::debug!(
                            stream_id = %stream_id,
                            "stream worker closed during backpressure; dropping chunk"
                        );
                    }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!(
                        stream_id = %stream_id,
                        "stream worker closed (unregistered/shutdown); dropping chunk"
                    );
                }
            },
        }
    }

    /// Gracefully shut down every worker: cancel queued work, let in-flight
    /// callbacks finish, then join all worker tasks (bounded, else abort).
    pub(crate) async fn shutdown(&self) {
        self.shutdown.cancel();

        // Drain the worker map, taking ownership of the handles.
        let keys: Vec<String> = self.workers.iter().map(|e| e.key().clone()).collect();
        let mut handles = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some((_, worker)) = self.workers.remove(&key) {
                handles.push(worker.handle);
            }
        }

        if handles.is_empty() {
            return;
        }

        let abort_handles: Vec<_> = handles.iter().map(|h| h.abort_handle()).collect();
        let joined = futures_util::future::join_all(handles);

        match tokio::time::timeout(SHUTDOWN_JOIN_TIMEOUT, joined).await {
            Ok(_) => {
                tracing::debug!("data stream workers joined on shutdown");
            }
            Err(_) => {
                for abort in abort_handles {
                    abort.abort();
                }
                tracing::error!(
                    timeout_secs = SHUTDOWN_JOIN_TIMEOUT.as_secs(),
                    "data stream workers did not finish before timeout; aborted"
                );
            }
        }
    }

    /// Get or lazily create the sender for `stream_id`'s worker.
    ///
    /// Returns a cloned bounded sender; the DashMap guard is never held across
    /// an `await` (the caller may block on `send().await`).
    fn worker_tx(
        &self,
        stream_id: &str,
        callback: DataStreamCallback,
    ) -> mpsc::Sender<QueuedChunk> {
        if let Some(worker) = self.workers.get(stream_id) {
            return worker.tx.clone();
        }

        let entry = self
            .workers
            .entry(stream_id.to_string())
            .or_insert_with(|| {
                let (tx, rx) = mpsc::channel(self.queue_depth);
                let handle = tokio::spawn(Self::worker_loop(
                    stream_id.to_string(),
                    rx,
                    callback,
                    self.shutdown.clone(),
                    self.panic_count.clone(),
                ));
                StreamWorker { tx, handle }
            });
        entry.tx.clone()
    }

    /// Per-stream worker loop: drain the queue in order, run-to-completion.
    async fn worker_loop(
        stream_id: String,
        mut rx: mpsc::Receiver<QueuedChunk>,
        callback: DataStreamCallback,
        shutdown: CancellationToken,
        panic_count: Arc<AtomicU64>,
    ) {
        loop {
            tokio::select! {
                biased;
                // Shutdown wins: drop queued chunks and stop. An in-flight
                // callback below is not interrupted mid-await; cancellation is
                // only observed between chunks.
                _ = shutdown.cancelled() => {
                    tracing::debug!(
                        stream_id = %stream_id,
                        "data stream worker cancelled; dropping queued chunks"
                    );
                    break;
                }
                maybe = rx.recv() => match maybe {
                    Some(queued) => {
                        Self::run_callback(&stream_id, &callback, queued, &panic_count).await;
                    }
                    // All senders dropped (unregister): queue drained, exit.
                    None => {
                        tracing::debug!(
                            stream_id = %stream_id,
                            "data stream worker drained; exiting"
                        );
                        break;
                    }
                }
            }
        }
    }

    /// Invoke a single callback with panic isolation; errors and panics are
    /// logged and counted, and the worker proceeds to the next chunk.
    async fn run_callback(
        stream_id: &str,
        callback: &DataStreamCallback,
        queued: QueuedChunk,
        panic_count: &AtomicU64,
    ) {
        let QueuedChunk { chunk, sender_id } = queued;
        let sequence = chunk.sequence;
        let fut = callback(chunk, sender_id);
        match AssertUnwindSafe(fut).catch_unwind().await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::error!(
                    stream_id = %stream_id,
                    sequence,
                    error = ?e,
                    "❌ data stream callback returned error; continuing"
                );
            }
            Err(panic_payload) => {
                let count = panic_count.fetch_add(1, Ordering::Relaxed) + 1;
                let info = panic_message(panic_payload);
                tracing::error!(
                    stream_id = %stream_id,
                    sequence,
                    panic = %info,
                    panic_total = count,
                    "❌ data stream callback panicked; isolated, continuing"
                );
            }
        }
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    }
}

#[cfg(test)]
#[path = "data_stream_registry_tests.rs"]
mod tests;
