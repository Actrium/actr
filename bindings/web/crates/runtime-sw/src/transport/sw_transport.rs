//! Transport implementation for the Service Worker runtime.
//!
//! This wrapper:
//! - Manages the WebSocket connection pool
//! - Manages the PostMessage channel to the DOM side
//! - Routes messages automatically based on `PayloadType`
//! - Forwards `MEDIA_RTP` traffic to the DOM side

use super::lane::DataLane;
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
// No `tokio` dependency here because it is not supported in the web runtime.

use super::websocket::WebSocketLaneBuilder;

/// Transport implementation for the Service Worker side.
#[allow(dead_code)]
pub struct SwTransport {
    /// Local ID.
    local_id: String,

    /// WebSocket pool: `Dest -> (DataLane, ConnectionState)`.
    websocket_pool: Arc<DashMap<Dest, (DataLane, ConnectionState)>>,

    /// DOM channel carried over PostMessage.
    dom_channel: Arc<Mutex<Option<DataLane>>>,

    /// Connection strategy.
    strategy: ConnectionStrategy,

    /// Runtime statistics.
    stats: Arc<Mutex<TransportStats>>,

    /// Aggregated receive channel.
    rx: Arc<Mutex<mpsc::UnboundedReceiver<(Dest, PayloadType, Bytes)>>>,
    tx: mpsc::UnboundedSender<(Dest, PayloadType, Bytes)>,
}

impl SwTransport {
    /// Create a new `SwTransport`.
    pub fn new(local_id: String, strategy: Option<ConnectionStrategy>) -> Self {
        let (tx, rx) = mpsc::unbounded();

        Self {
            local_id,
            websocket_pool: Arc::new(DashMap::new()),
            dom_channel: Arc::new(Mutex::new(None)),
            strategy: strategy.unwrap_or_default(),
            stats: Arc::new(Mutex::new(TransportStats::default())),
            rx: Arc::new(Mutex::new(rx)),
            tx,
        }
    }

    /// Set the DOM channel.
    ///
    /// Called when the DOM side connects to the SW through a `MessagePort`.
    pub fn set_dom_channel(&self, lane: DataLane) -> WebResult<()> {
        let mut dom_channel = self.dom_channel.lock();
        *dom_channel = Some(lane);

        log::info!("[SwTransport] DOM channel established");

        // Start the DOM receive loop.
        self.start_dom_receiver();

        Ok(())
    }

    /// Send a message.
    pub async fn send(&self, dest: &Dest, payload_type: PayloadType, data: Bytes) -> WebResult<()> {
        log::trace!(
            "[SwTransport] send: dest={:?}, payload_type={:?}, size={} bytes",
            dest,
            payload_type,
            data.len()
        );

        // Routing policy.
        match payload_type {
            // RPC and stream traffic stays in the SW and uses WebSocket.
            PayloadType::RpcReliable
            | PayloadType::RpcSignal
            | PayloadType::StreamReliable
            | PayloadType::StreamLatencyFirst => {
                let lane = self.get_or_create_websocket(dest).await?;
                lane.send(data.clone()).await?;

                // Update stats.
                let mut stats = self.stats.lock();
                stats.bytes_sent += data.len() as u64;
                stats.messages_sent += 1;
            }

            // MEDIA_RTP must be forwarded to the DOM side.
            PayloadType::MediaRtp => {
                self.forward_to_dom(dest, payload_type, data).await?;
            }
        }

        Ok(())
    }

    /// Receive a message.
    #[allow(clippy::await_holding_lock)] // Single-threaded wasm: no contention risk
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

    /// Get transport statistics.
    pub fn stats(&self) -> TransportStats {
        self.stats.lock().clone()
    }

    /// Get or create a WebSocket connection.
    ///
    /// Simplified implementation: create directly and let `DashMap` handle concurrency.
    async fn get_or_create_websocket(&self, dest: &Dest) -> WebResult<DataLane> {
        // 1. Fast path: reuse an existing connected lane.
        if let Some(entry) = self.websocket_pool.get(dest) {
            let (lane, state) = entry.value();
            if *state == ConnectionState::Connected {
                return Ok(lane.clone());
            }
        }

        // 2. Create a new connection.
        let lane = self.create_websocket_connection(dest).await?;

        // Mark it as connected.
        self.websocket_pool
            .insert(dest.clone(), (lane.clone(), ConnectionState::Connected));

        log::info!("[SwTransport] WebSocket connected: {:?}", dest);

        // Start the receive loop.
        self.start_websocket_receiver(dest.clone(), lane.clone());

        Ok(lane)
    }

    /// Create a WebSocket connection.
    async fn create_websocket_connection(&self, dest: &Dest) -> WebResult<DataLane> {
        let url = dest.to_websocket_url()?;

        log::debug!("[SwTransport] Creating WebSocket connection to: {}", url);

        let lane = WebSocketLaneBuilder::new(url, PayloadType::RpcReliable)
            .build()
            .await?;

        Ok(lane)
    }

    /// Forward a message to the DOM side.
    async fn forward_to_dom(
        &self,
        dest: &Dest,
        payload_type: PayloadType,
        data: Bytes,
    ) -> WebResult<()> {
        let lane_opt = {
            let dom_channel = self.dom_channel.lock();
            dom_channel.clone()
        };

        if let Some(lane) = lane_opt.as_ref() {
            let forward_msg = ForwardMessage::new(dest.clone(), payload_type, data);
            let serialized = forward_msg.serialize()?;

            lane.send(serialized).await?;

            log::trace!(
                "[SwTransport] Forwarded to DOM: dest={:?}, payload_type={:?}",
                dest,
                payload_type
            );

            Ok(())
        } else {
            Err(WebError::Transport(
                "DOM channel not available, cannot forward MEDIA_RTP".to_string(),
            ))
        }
    }

    /// Start the WebSocket receive loop.
    fn start_websocket_receiver(&self, dest: Dest, lane: DataLane) {
        let tx = self.tx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            loop {
                match lane.recv().await {
                    Some(data) => {
                        // Determine the payload type.
                        // This is simplified and currently uses the lane's configured type.
                        let payload_type = lane.payload_type();

                        if let Err(e) = tx.unbounded_send((dest.clone(), payload_type, data)) {
                            log::error!(
                                "[SwTransport] Failed to forward received message: {:?}",
                                e
                            );
                            break;
                        }
                    }
                    None => {
                        log::warn!("[SwTransport] WebSocket receiver closed: {:?}", dest);
                        break;
                    }
                }
            }
        });
    }

    /// Start the DOM receive loop.
    fn start_dom_receiver(&self) {
        let dom_channel = self.dom_channel.clone();
        let tx = self.tx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let lane = {
                    let channel = dom_channel.lock();
                    channel.as_ref().cloned()
                };

                if let Some(lane) = lane {
                    match lane.recv().await {
                        Some(data) => {
                            // Parse the forwarded message.
                            match ForwardMessage::deserialize(&data) {
                                Ok(forward_msg) => {
                                    log::trace!(
                                        "[SwTransport] Received from DOM: dest={:?}, payload_type={:?}",
                                        forward_msg.dest,
                                        forward_msg.payload_type
                                    );

                                    if let Err(e) = tx.unbounded_send((
                                        forward_msg.dest,
                                        forward_msg.payload_type,
                                        forward_msg.data,
                                    )) {
                                        log::error!(
                                            "[SwTransport] Failed to forward DOM message: {:?}",
                                            e
                                        );
                                    }
                                }
                                Err(e) => {
                                    log::error!(
                                        "[SwTransport] Failed to parse forward message: {:?}",
                                        e
                                    );
                                }
                            }
                        }
                        None => {
                            log::warn!("[SwTransport] DOM receiver closed");
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
        self.websocket_pool.remove(dest);
        log::info!("[SwTransport] Disconnected: {:?}", dest);
        Ok(())
    }

    /// Get the connection state for a destination.
    pub fn connection_state(&self, dest: &Dest) -> ConnectionState {
        self.websocket_pool
            .get(dest)
            .map(|entry| entry.value().1)
            .unwrap_or(ConnectionState::Disconnected)
    }
}
