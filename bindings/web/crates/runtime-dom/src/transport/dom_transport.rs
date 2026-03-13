//! DOM transport implementation.
//!
//! Unified transport wrapper responsible for:
//! - Managing the WebRTC P2P connection pool (DataChannel + MediaTrack)
//! - Managing the PostMessage channel to the Service Worker
//! - Applying concurrent connection attempts (P2P + WebSocket fallback)
//! - Preferring the connection that becomes ready first
//! - Automatic routing and forwarding
//! - Fast Path integration

use super::lane::DataLane;
use crate::fastpath::{MediaFrameHandlerRegistry, StreamHandlerRegistry};
use crate::keepalive::ServiceWorkerKeepalive;
use actr_web_common::{
    ConnectionState, ConnectionStrategy, Dest, ForwardMessage, PayloadType, TransportStats,
    WebError, WebResult,
};
use bytes::Bytes;
use dashmap::DashMap;
use futures::StreamExt;
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;

/// Connection type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionType {
    /// P2P (WebRTC)
    P2P,
    /// WebSocket via the Service Worker.
    WebSocket,
}

/// Connection information for a destination.
#[allow(dead_code)]
struct DestConnection {
    /// Primary connection, either DataChannel or a WebSocket routed through the SW.
    primary: Option<DataLane>,

    /// Connection type.
    conn_type: ConnectionType,

    /// Connection state.
    state: ConnectionState,

    /// MediaTrack lanes when the connection is P2P.
    media_tracks: Vec<DataLane>,
}

/// Transport implementation for the DOM side.
#[allow(dead_code)]
pub struct DomTransport {
    /// Local ID.
    local_id: String,

    /// Connection pool keyed by destination.
    connections: Arc<DashMap<Dest, DestConnection>>,

    /// Service Worker PostMessage channel.
    sw_channel: Arc<Mutex<Option<DataLane>>>,

    /// Fast Path Registries
    stream_registry: Arc<StreamHandlerRegistry>,
    media_registry: Arc<MediaFrameHandlerRegistry>,

    /// Keepalive
    keepalive: Arc<Mutex<Option<ServiceWorkerKeepalive>>>,

    /// Connection strategy.
    strategy: ConnectionStrategy,

    /// Transport statistics.
    stats: Arc<Mutex<TransportStats>>,

    /// Receive channel.
    rx: Arc<Mutex<mpsc::UnboundedReceiver<(Dest, PayloadType, Bytes)>>>,
    tx: mpsc::UnboundedSender<(Dest, PayloadType, Bytes)>,
}

impl DomTransport {
    /// Create a new DomTransport.
    pub fn new(local_id: String, strategy: Option<ConnectionStrategy>) -> Self {
        let (tx, rx) = mpsc::unbounded();

        Self {
            local_id,
            connections: Arc::new(DashMap::new()),
            sw_channel: Arc::new(Mutex::new(None)),
            stream_registry: Arc::new(StreamHandlerRegistry::new()),
            media_registry: Arc::new(MediaFrameHandlerRegistry::new()),
            keepalive: Arc::new(Mutex::new(None)),
            strategy: strategy.unwrap_or_default(),
            stats: Arc::new(Mutex::new(TransportStats::default())),
            rx: Arc::new(Mutex::new(rx)),
            tx,
        }
    }

    /// Set the Service Worker channel and start keepalive.
    pub fn set_sw_channel(&self, lane: DataLane) -> WebResult<()> {
        // Create the keepalive helper.
        let keepalive = ServiceWorkerKeepalive::new(Arc::new(lane.clone()), None);
        keepalive.start();

        {
            let mut sw_channel = self.sw_channel.lock();
            *sw_channel = Some(lane);
        }

        {
            let mut ka = self.keepalive.lock();
            *ka = Some(keepalive);
        }

        log::info!("[DomTransport] SW channel established with keepalive");

        // Start the Service Worker receive loop.
        self.start_sw_receiver();

        Ok(())
    }

    /// Send a message.
    pub async fn send(&self, dest: &Dest, payload_type: PayloadType, data: Bytes) -> WebResult<()> {
        log::trace!(
            "[DomTransport] send: dest={:?}, payload_type={:?}, size={} bytes",
            dest,
            payload_type,
            data.len()
        );

        match payload_type {
            // RPC is forwarded to the Service Worker through the State Path.
            PayloadType::RpcReliable | PayloadType::RpcSignal => {
                self.forward_to_sw(dest, payload_type, data).await?;
            }

            // STREAM payloads are handled on the DOM side.
            PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                let data_len = data.len();

                // Try to use the P2P connection.
                if let Some(lane) = self.get_connection(dest).await {
                    lane.send(data).await?;
                } else {
                    // Fall back to the Service Worker over WebSocket.
                    log::warn!("[DomTransport] P2P not available, fallback to SW for STREAM");
                    self.forward_to_sw(dest, payload_type, data).await?;
                }

                // Update statistics.
                let mut stats = self.stats.lock();
                stats.bytes_sent += data_len as u64;
                stats.messages_sent += 1;
            }

            PayloadType::MediaRtp => {
                // MEDIA_RTP must use MediaTrack over P2P.
                if let Some(lane) = self.get_connection(dest).await {
                    let data_len = data.len();
                    lane.send(data).await?;

                    let mut stats = self.stats.lock();
                    stats.bytes_sent += data_len as u64;
                    stats.messages_sent += 1;
                } else {
                    return Err(WebError::Transport(
                        "No P2P connection available for MEDIA_RTP".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Receive a message.
    #[allow(clippy::await_holding_lock)] // wasm single-threaded: parking_lot Mutex is not contended
    pub async fn recv(&self) -> Option<(Dest, PayloadType, Bytes)> {
        let mut rx = self.rx.lock();
        let msg = rx.next().await;

        if let Some((_, _, ref data)) = msg {
            let mut stats = self.stats.lock();
            stats.bytes_received += data.len() as u64;
            stats.messages_received += 1;
        }

        msg
    }

    /// Connect to a destination using the configured strategy.
    ///
    /// **Core strategy**:
    /// 1. Try P2P and WebSocket concurrently through the Service Worker
    /// 2. Prefer the connection that becomes ready first
    /// 3. If both succeed, prefer P2P when it has higher priority
    pub async fn connect(&self, dest: &Dest) -> WebResult<()> {
        // Check whether we are already connected.
        if let Some(entry) = self.connections.get(dest) {
            if entry.state == ConnectionState::Connected {
                log::debug!("[DomTransport] Already connected to {:?}", dest);
                return Ok(());
            }
        }

        // Mark the destination as connecting.
        self.connections.insert(
            dest.clone(),
            DestConnection {
                primary: None,
                conn_type: ConnectionType::P2P,
                state: ConnectionState::Connecting,
                media_tracks: Vec::new(),
            },
        );

        // Attempt the connection.
        let result = if self.strategy.concurrent_attempts {
            self.concurrent_connect(dest).await
        } else {
            self.sequential_connect(dest).await
        };

        match result {
            Ok(conn) => {
                // Update the state to connected.
                self.connections.insert(dest.clone(), conn);

                log::info!("[DomTransport] Connected to {:?}", dest);

                Ok(())
            }
            Err(e) => {
                // Mark the attempt as failed.
                self.connections.remove(dest);

                Err(e)
            }
        }
    }

    /// Concurrent connection strategy that prefers whichever becomes ready first.
    ///
    /// Attempts both:
    /// 1. P2P (WebRTC DataChannel)
    /// 2. WebSocket (through the Service Worker)
    ///
    /// **Ready-first preference**: use whichever connects first.
    /// **Priority override**: if both succeed, prefer P2P.
    async fn concurrent_connect(&self, dest: &Dest) -> WebResult<DestConnection> {
        use futures::future::FutureExt;

        log::debug!("[DomTransport] Concurrent connect to {:?}", dest);

        // Create both connection tasks.
        let p2p_future = self.create_p2p_connection(dest).fuse();
        let websocket_future = self.create_websocket_fallback(dest).fuse();

        futures::pin_mut!(p2p_future, websocket_future);

        // Use `select!` to race both attempts.
        let mut p2p_result = None;
        let mut ws_result = None;

        // First round: wait for the first completion.
        futures::select! {
            result = p2p_future => {
                p2p_result = Some(result);
            }
            result = websocket_future => {
                ws_result = Some(result);
            }
        }

        // If P2P succeeds first, use it immediately.
        if let Some(Ok(conn)) = p2p_result {
            log::info!("[DomTransport] P2P connected first (concurrent mode)");
            return Ok(conn);
        }

        // If WebSocket succeeds first, check whether P2P also succeeds shortly after.
        if let Some(Ok(ws_conn)) = ws_result {
            // Keep waiting briefly for P2P if it has higher priority.
            if self.strategy.p2p_priority > self.strategy.websocket_priority {
                // Wait up to 100 ms to see whether P2P succeeds.
                let timeout = async {
                    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::new(
                        &mut |resolve, _| {
                            let window = web_sys::window().unwrap();
                            window
                                .set_timeout_with_callback_and_timeout_and_arguments_0(
                                    &resolve, 100,
                                )
                                .unwrap();
                        },
                    ))
                    .await
                    .ok();
                };

                futures::select! {
                    result = p2p_future => {
                        if let Ok(p2p_conn) = result {
                            log::info!("[DomTransport] P2P also succeeded, using P2P (priority)");
                            return Ok(p2p_conn);
                        }
                    }
                    _ = timeout.fuse() => {
                        log::info!("[DomTransport] WebSocket connected first, P2P timeout");
                    }
                }
            }

            log::info!("[DomTransport] WebSocket connected (concurrent mode)");
            return Ok(ws_conn);
        }

        // Both attempts failed.
        Err(WebError::Transport(format!(
            "Failed to connect to {:?}: both P2P and WebSocket failed",
            dest
        )))
    }

    /// Sequential connection strategy: try P2P first, then WebSocket.
    async fn sequential_connect(&self, dest: &Dest) -> WebResult<DestConnection> {
        log::debug!("[DomTransport] Sequential connect to {:?}", dest);

        // Try P2P first.
        match self.create_p2p_connection(dest).await {
            Ok(conn) => {
                log::info!("[DomTransport] P2P connected (sequential mode)");
                Ok(conn)
            }
            Err(e) => {
                log::warn!("[DomTransport] P2P failed: {:?}, trying WebSocket", e);

                // Fall back to WebSocket.
                self.create_websocket_fallback(dest).await
            }
        }
    }

    /// Create a P2P connection using WebRTC DataChannel and MediaTrack.
    async fn create_p2p_connection(&self, _dest: &Dest) -> WebResult<DestConnection> {
        // TODO: implement WebRTC signaling and PeerConnection creation.
        // This requires:
        // 1. SDP offer/answer exchange
        // 2. ICE candidate exchange
        // 3. DataChannel creation
        // 4. MediaTrack creation when needed

        // Not implemented yet. Planned for Phase 4.
        Err(WebError::Transport(
            "P2P connection not implemented yet (Phase 4)".to_string(),
        ))
    }

    /// Create the WebSocket fallback through the Service Worker.
    async fn create_websocket_fallback(&self, dest: &Dest) -> WebResult<DestConnection> {
        log::debug!("[DomTransport] Creating WebSocket fallback for {:?}", dest);

        // The WebSocket is established by the Service Worker.
        // No local lane is required because all messages are forwarded through the SW.

        Ok(DestConnection {
            primary: None, // Forwarded through the SW, so no local lane is needed.
            conn_type: ConnectionType::WebSocket,
            state: ConnectionState::Connected,
            media_tracks: Vec::new(),
        })
    }

    /// Get the active connection.
    async fn get_connection(&self, dest: &Dest) -> Option<DataLane> {
        if let Some(entry) = self.connections.get(dest) {
            if entry.state == ConnectionState::Connected {
                return entry.primary.clone();
            }
        }

        None
    }

    /// Forward a message to the Service Worker.
    #[allow(clippy::await_holding_lock)] // wasm single-threaded: parking_lot Mutex is not contended
    async fn forward_to_sw(
        &self,
        dest: &Dest,
        payload_type: PayloadType,
        data: Bytes,
    ) -> WebResult<()> {
        let sw_channel = self.sw_channel.lock();

        if let Some(lane) = sw_channel.as_ref() {
            let forward_msg = ForwardMessage::new(dest.clone(), payload_type, data);
            let serialized = forward_msg.serialize()?;

            lane.send(serialized).await?;

            log::trace!(
                "[DomTransport] Forwarded to SW: dest={:?}, payload_type={:?}",
                dest,
                payload_type
            );

            Ok(())
        } else {
            Err(WebError::Transport("SW channel not available".to_string()))
        }
    }

    /// Start the Service Worker receive loop.
    fn start_sw_receiver(&self) {
        let sw_channel = self.sw_channel.clone();
        let tx = self.tx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let lane = {
                    let channel = sw_channel.lock();
                    channel.as_ref().cloned()
                };

                if let Some(lane) = lane {
                    match lane.recv().await {
                        Some(data) => match ForwardMessage::deserialize(&data) {
                            Ok(forward_msg) => {
                                log::trace!(
                                    "[DomTransport] Received from SW: dest={:?}, payload_type={:?}",
                                    forward_msg.dest,
                                    forward_msg.payload_type
                                );

                                if let Err(e) = tx.unbounded_send((
                                    forward_msg.dest,
                                    forward_msg.payload_type,
                                    forward_msg.data,
                                )) {
                                    log::error!(
                                        "[DomTransport] Failed to forward SW message: {:?}",
                                        e
                                    );
                                }
                            }
                            Err(e) => {
                                log::error!(
                                    "[DomTransport] Failed to parse forward message: {:?}",
                                    e
                                );
                            }
                        },
                        None => {
                            log::warn!("[DomTransport] SW receiver closed");
                            break;
                        }
                    }
                } else {
                    break;
                }
            }
        });
    }

    /// Disconnect a destination.
    pub async fn disconnect(&self, dest: &Dest) -> WebResult<()> {
        self.connections.remove(dest);
        log::info!("[DomTransport] Disconnected: {:?}", dest);
        Ok(())
    }

    /// Get the connection state for a destination.
    pub fn connection_state(&self, dest: &Dest) -> ConnectionState {
        self.connections
            .get(dest)
            .map(|entry| entry.state)
            .unwrap_or(ConnectionState::Disconnected)
    }

    /// Return transport statistics.
    pub fn stats(&self) -> TransportStats {
        self.stats.lock().clone()
    }

    /// Get the stream registry.
    pub fn stream_registry(&self) -> &Arc<StreamHandlerRegistry> {
        &self.stream_registry
    }

    /// Get the media registry.
    pub fn media_registry(&self) -> &Arc<MediaFrameHandlerRegistry> {
        &self.media_registry
    }
}
