//! DataLane - Business data transport channel
//!
//! DataLane is the core abstraction of the transport layer for message/data transmission.
//! Note: MediaTrack uses a separate MediaFrameRegistry path, not DataLane.
//!
//! ## Design Philosophy
//!
//! ```text
//! DataLane features:
//!   ✓ enum type with 3 variants (WebRtcDataChannel, Mpsc, WebSocket)
//!   ✓ Unified send/recv API for data messages
//!   ✓ Cloneable (uses Arc internally for sharing)
//!   ✓ Multi-consumer pattern (shared receive channel)
//! ```

use super::error::{NetworkError, NetworkResult};
use actr_protocol::PayloadType;
use futures_util::SinkExt;
use futures_util::stream::SplitSink;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use webrtc::data_channel::RTCDataChannel;

/// Type alias for WebSocket sink (shared across all PayloadTypes)
type WsSink = Arc<Mutex<Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>>>>;

/// DataLane - Data transport channel
///
/// Each DataLane represents a specific transport path for data/message transmission.
/// MediaTrack uses a separate path via MediaFrameRegistry, not DataLane.
#[derive(Clone)]
pub enum DataLane {
    /// WebRTC DataChannel Lane
    ///
    /// For transmitting messages via WebRTC DataChannel
    WebRtcDataChannel {
        /// Underlying DataChannel
        data_channel: Arc<RTCDataChannel>,

        /// Receive channel (shared, uses Bytes for zero-copy)
        rx: Arc<Mutex<mpsc::Receiver<bytes::Bytes>>>,
    },

    /// Mpsc Lane
    ///
    /// For intra-process communication (Inproc transport)
    ///
    /// Note: directly passes RpcEnvelope objects, zero serialization
    Mpsc {
        /// PayloadType identifier
        payload_type: PayloadType,

        /// Send channel (directly passes RpcEnvelope)
        tx: mpsc::Sender<actr_protocol::RpcEnvelope>,

        /// Receive channel (shared)
        rx: Arc<Mutex<mpsc::Receiver<actr_protocol::RpcEnvelope>>>,
    },

    /// WebSocket Lane
    ///
    /// For business data transmission in C/S architecture
    WebSocket {
        /// Shared Sink (all PayloadTypes share the same WebSocket connection)
        /// Uses Option to support lazy initialization
        sink: WsSink,

        /// PayloadType identifier (used to add message header when sending)
        payload_type: PayloadType,

        /// Receive channel (independent, routed by dispatcher, uses Bytes for zero-copy)
        rx: Arc<Mutex<mpsc::Receiver<bytes::Bytes>>>,
    },
}

impl DataLane {
    /// Send message
    ///
    /// # Arguments
    /// - `data`: message data (uses Bytes for zero-copy)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use bytes::Bytes;
    /// data_lane.send(Bytes::from_static(b"hello")).await?;
    /// ```
    pub async fn send(&self, data: bytes::Bytes) -> NetworkResult<()> {
        match self {
            DataLane::WebRtcDataChannel { data_channel, .. } => {
                use webrtc::data_channel::data_channel_state::RTCDataChannelState;

                // Wait for DataChannel to open (max 5 seconds)
                let start = tokio::time::Instant::now();
                loop {
                    let state = data_channel.ready_state();
                    if state == RTCDataChannelState::Open {
                        break;
                    }
                    if state == RTCDataChannelState::Closed || state == RTCDataChannelState::Closing
                    {
                        return Err(NetworkError::DataChannelError(format!(
                            "DataChannel closed: {state:?}"
                        )));
                    }
                    if start.elapsed() > std::time::Duration::from_secs(5) {
                        return Err(NetworkError::DataChannelError(format!(
                            "DataChannel open timeout: {state:?}"
                        )));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }

                // Zero-copy: directly use the passed Bytes
                data_channel
                    .send(&data)
                    .await
                    .map_err(|e| NetworkError::DataChannelError(format!("Send failed: {e}")))?;

                tracing::trace!("📤 WebRTC DataChannel sent {} bytes", data.len());
                Ok(())
            }

            DataLane::Mpsc { .. } => {
                // Mpsc DataLane should use send_envelope() instead of send(bytes)
                Err(NetworkError::InvalidOperation(
                    "Mpsc DataLane requires send_envelope(), not send(bytes)".to_string(),
                ))
            }

            DataLane::WebSocket {
                sink, payload_type, ..
            } => {
                // 1. Encapsulate message (add PayloadType header)
                let mut buf = Vec::with_capacity(5 + data.len());

                // 1 byte: payload_type
                buf.push(*payload_type as u8);

                // 4 bytes: data length (big-endian)
                let len = data.len() as u32;
                buf.extend_from_slice(&len.to_be_bytes());

                // N bytes: data (copy from Bytes to Vec)
                buf.extend_from_slice(&data);

                // 2. Send to WebSocket
                let mut sink_opt = sink.lock().await;
                if let Some(s) = sink_opt.as_mut() {
                    s.send(WsMessage::Binary(buf.into())).await.map_err(|e| {
                        NetworkError::SendError(format!("WebSocket send failed: {e}"))
                    })?;

                    tracing::trace!(
                        "📤 WebSocket sent {} bytes (type={:?})",
                        data.len(),
                        payload_type
                    );
                    Ok(())
                } else {
                    Err(NetworkError::ConnectionError(
                        "WebSocket not connected".to_string(),
                    ))
                }
            }
        }
    }

    /// Send RpcEnvelope (Inproc only, zero serialization)
    ///
    /// # Arguments
    /// - `envelope`: RpcEnvelope object
    ///
    /// # Description
    /// This method is only for `DataLane::Mpsc`, directly passing RpcEnvelope objects,
    /// without serialization/deserialization, achieving zero-copy intra-process communication.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use actr_protocol::RpcEnvelope;
    /// let envelope = RpcEnvelope { /* ... */ };
    /// data_lane.send_envelope(envelope).await?;
    /// ```
    pub async fn send_envelope(&self, envelope: actr_protocol::RpcEnvelope) -> NetworkResult<()> {
        match self {
            DataLane::Mpsc { tx, .. } => {
                tx.send(envelope)
                    .await
                    .map_err(|_| NetworkError::ChannelClosed("Mpsc channel closed".to_string()))?;

                tracing::trace!("📤 Mpsc sent RpcEnvelope");
                Ok(())
            }
            _ => Err(NetworkError::InvalidOperation(
                "send_envelope() only supports Mpsc DataLane".to_string(),
            )),
        }
    }

    /// Receive message
    ///
    /// Blocks until a message is received or the channel is closed.
    ///
    /// # Returns
    /// - `Ok(Bytes)`: received message data (zero-copy)
    /// - `Err`: channel closed or other error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let data = data_lane.recv().await?;
    /// println!("Received {} bytes", data.len());
    /// ```
    pub async fn recv(&self) -> NetworkResult<bytes::Bytes> {
        match self {
            DataLane::WebRtcDataChannel { rx, .. } | DataLane::WebSocket { rx, .. } => {
                let mut receiver = rx.lock().await;
                receiver.recv().await.ok_or_else(|| {
                    NetworkError::ChannelClosed("DataLane receiver closed".to_string())
                })
            }
            DataLane::Mpsc { .. } => {
                // Mpsc DataLane should use recv_envelope() instead of recv()
                Err(NetworkError::InvalidOperation(
                    "Mpsc DataLane requires recv_envelope(), not recv()".to_string(),
                ))
            }
        }
    }

    /// Receive RpcEnvelope (Inproc only)
    ///
    /// # Returns
    /// - `Ok(RpcEnvelope)`: received message object
    /// - `Err`: channel closed
    ///
    /// # Description
    /// This method is only for `DataLane::Mpsc`, directly receiving RpcEnvelope objects, zero-copy.
    pub async fn recv_envelope(&self) -> NetworkResult<actr_protocol::RpcEnvelope> {
        match self {
            DataLane::Mpsc { rx, .. } => {
                let mut receiver = rx.lock().await;
                receiver
                    .recv()
                    .await
                    .ok_or_else(|| NetworkError::ChannelClosed("Mpsc channel closed".to_string()))
            }
            _ => Err(NetworkError::InvalidOperation(
                "recv_envelope() only supports Mpsc DataLane".to_string(),
            )),
        }
    }

    /// Try to receive message (non-blocking)
    ///
    /// # Returns
    /// - `Ok(Some(data))`: received message (zero-copy)
    /// - `Ok(None)`: no message available
    /// - `Err`: channel closed or other error
    pub async fn try_recv(&self) -> NetworkResult<Option<bytes::Bytes>> {
        match self {
            DataLane::WebRtcDataChannel { rx, .. } | DataLane::WebSocket { rx, .. } => {
                let mut receiver = rx.lock().await;
                match receiver.try_recv() {
                    Ok(data) => Ok(Some(data)),
                    Err(mpsc::error::TryRecvError::Empty) => Ok(None),
                    Err(mpsc::error::TryRecvError::Disconnected) => Err(
                        NetworkError::ChannelClosed("Lane receiver closed".to_string()),
                    ),
                }
            }
            DataLane::Mpsc { .. } => {
                // Mpsc Lane should use try_recv_envelope()
                Err(NetworkError::InvalidOperation(
                    "Mpsc Lane requires try_recv_envelope(), not try_recv()".to_string(),
                ))
            }
        }
    }

    /// Get DataLane type name (for logging)
    #[inline]
    pub fn lane_type(&self) -> &'static str {
        match self {
            DataLane::WebRtcDataChannel { .. } => "WebRtcDataChannel",
            DataLane::Mpsc { .. } => "Mpsc",
            DataLane::WebSocket { .. } => "WebSocket",
        }
    }
}

impl std::fmt::Debug for DataLane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataLane::WebRtcDataChannel { .. } => write!(f, "DataLane::WebRtcDataChannel(..)"),
            DataLane::Mpsc { .. } => write!(f, "DataLane::Mpsc(..)"),
            DataLane::WebSocket { payload_type, .. } => {
                write!(f, "DataLane::WebSocket(type={payload_type:?})")
            }
        }
    }
}

/// DataLane factory methods
impl DataLane {
    /// Create Mpsc DataLane (accepts plain Receiver)
    ///
    /// # Arguments
    /// - `payload_type`: PayloadType identifier
    /// - `tx`: send channel (directly passes RpcEnvelope)
    /// - `rx`: receive channel (automatically wrapped in Arc<Mutex<>>)
    #[inline]
    pub fn mpsc(
        payload_type: PayloadType,
        tx: mpsc::Sender<actr_protocol::RpcEnvelope>,
        rx: mpsc::Receiver<actr_protocol::RpcEnvelope>,
    ) -> Self {
        DataLane::Mpsc {
            payload_type,
            tx,
            rx: Arc::new(Mutex::new(rx)),
        }
    }

    /// Create Mpsc DataLane (accepts shared Receiver)
    ///
    /// # Arguments
    /// - `payload_type`: PayloadType identifier
    /// - `tx`: send channel (directly passes RpcEnvelope)
    /// - `rx`: shared receive channel
    #[inline]
    pub fn mpsc_shared(
        payload_type: PayloadType,
        tx: mpsc::Sender<actr_protocol::RpcEnvelope>,
        rx: Arc<Mutex<mpsc::Receiver<actr_protocol::RpcEnvelope>>>,
    ) -> Self {
        DataLane::Mpsc {
            payload_type,
            tx,
            rx,
        }
    }

    /// Create WebRTC DataChannel DataLane
    ///
    /// # Arguments
    /// - `data_channel`: DataChannel reference
    /// - `rx`: receive channel (Bytes zero-copy)
    #[inline]
    pub fn webrtc_data_channel(
        data_channel: Arc<RTCDataChannel>,
        rx: mpsc::Receiver<bytes::Bytes>,
    ) -> Self {
        DataLane::WebRtcDataChannel {
            data_channel,
            rx: Arc::new(Mutex::new(rx)),
        }
    }

    /// Create WebSocket DataLane
    ///
    /// # Arguments
    /// - `sink`: shared WebSocket Sink (may not be connected yet, uses Option)
    /// - `payload_type`: message type identifier
    /// - `rx`: receive channel (Bytes zero-copy)
    #[inline]
    pub fn websocket(
        sink: WsSink,
        payload_type: PayloadType,
        rx: mpsc::Receiver<bytes::Bytes>,
    ) -> Self {
        DataLane::WebSocket {
            sink,
            payload_type,
            rx: Arc::new(Mutex::new(rx)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[tokio::test]
    async fn test_mpsc_lane() {
        use actr_protocol::RpcEnvelope;

        let (tx, rx) = mpsc::channel(10);
        let lane = DataLane::mpsc(PayloadType::RpcReliable, tx.clone(), rx);

        // Send message (using RpcEnvelope)
        let envelope = RpcEnvelope {
            request_id: "test-1".to_string(),
            route_key: "test.route".to_string(),
            payload: Some(Bytes::from_static(b"hello")),
            traceparent: None,
            tracestate: None,
            metadata: vec![],
            timeout_ms: 30000,
            error: None,
        };
        lane.send_envelope(envelope.clone()).await.unwrap();

        // Receive message
        let received = lane.recv_envelope().await.unwrap();
        assert_eq!(received.request_id, "test-1");
        assert_eq!(received.payload, Some(Bytes::from_static(b"hello")));
    }

    #[tokio::test]
    async fn test_mpsc_lane_clone() {
        use actr_protocol::RpcEnvelope;

        let (tx, rx) = mpsc::channel(10);
        let lane = DataLane::mpsc(PayloadType::RpcReliable, tx.clone(), rx);

        // Clone lane
        let lane2 = lane.clone();

        // Send via lane
        let envelope = RpcEnvelope {
            request_id: "test-2".to_string(),
            route_key: "test.route".to_string(),
            payload: Some(Bytes::from_static(b"test")),
            traceparent: None,
            tracestate: None,
            metadata: vec![],
            timeout_ms: 30000,
            error: None,
        };
        lane.send_envelope(envelope.clone()).await.unwrap();

        // Receive via lane2
        let received = lane2.recv_envelope().await.unwrap();
        assert_eq!(received.request_id, "test-2");
        assert_eq!(received.payload, Some(Bytes::from_static(b"test")));
    }

    #[tokio::test]
    async fn test_mpsc_lane_with_shared_rx() {
        use actr_protocol::RpcEnvelope;

        let (tx, rx) = mpsc::channel(10);
        let rx_shared = Arc::new(Mutex::new(rx));

        // Use shared rx
        let lane = DataLane::mpsc_shared(PayloadType::RpcReliable, tx.clone(), rx_shared.clone());

        let envelope = RpcEnvelope {
            request_id: "test-3".to_string(),
            route_key: "test.route".to_string(),
            payload: Some(Bytes::from_static(b"shared")),
            traceparent: None,
            tracestate: None,
            metadata: vec![],
            timeout_ms: 30000,
            error: None,
        };
        lane.send_envelope(envelope.clone()).await.unwrap();

        let received = lane.recv_envelope().await.unwrap();
        assert_eq!(received.request_id, "test-3");
        assert_eq!(received.payload, Some(Bytes::from_static(b"shared")));
    }

    #[test]
    fn test_lane_type_name() {
        let (tx, rx) = mpsc::channel(10);
        let lane = DataLane::mpsc(PayloadType::RpcReliable, tx, rx);
        assert_eq!(lane.lane_type(), "Mpsc");
    }
}
