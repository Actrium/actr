//! WebSocket C/S Connection implementation

use crate::transport::{
    ConnType, DataLane, NetworkError, NetworkResult, WebSocketDataLane, WireHandle, WsSink,
};
use actr_protocol::PayloadType;
use async_trait::async_trait;
use futures_util::stream::SplitStream;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::http::Request as WsRequest;
use tokio_tungstenite::tungstenite::http::Uri as WsUri;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

/// WebSocket transmitting messagesprotocol
///
/// Forin single WebSocket Connect for multiple route reuse different Type'smessage。
///
/// ## Message format
///
/// ```text
/// [payload_type: 1 byte][data_len: 4 bytes][data: N bytes]
/// ```
/// Type alias for lane cache array (PayloadType index → cached DataLane)
type LaneCache<const N: usize> = Arc<RwLock<[Option<Arc<dyn DataLane>>; N]>>;

#[derive(Debug, Clone)]
struct TransportMessage {
    payload_type: PayloadType,
    data: Vec<u8>,
}

impl TransportMessage {
    /// frombytes stream decode
    fn decode(data: &[u8]) -> NetworkResult<Self> {
        if data.len() < 5 {
            return Err(NetworkError::DeserializationError(
                "WebSocket message too short".to_string(),
            ));
        }

        // Parse payload_type (must match proto enum values)
        let payload_type_raw = data[0];
        let payload_type = match payload_type_raw {
            0 => PayloadType::RpcReliable,
            1 => PayloadType::RpcSignal,
            2 => PayloadType::StreamReliable,
            3 => PayloadType::StreamLatencyFirst,
            4 => PayloadType::MediaRtp,
            _ => {
                return Err(NetworkError::DeserializationError(format!(
                    "Invalid payload_type: {payload_type_raw}"
                )));
            }
        };

        // Parse length
        let len = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;

        // Parse data
        if data.len() < 5 + len {
            return Err(NetworkError::DeserializationError(
                "WebSocket message data incomplete".to_string(),
            ));
        }

        let msg_data = data[5..5 + len].to_vec();

        Ok(Self {
            payload_type,
            data: msg_data,
        })
    }
}

/// WebSocketConnection - WebSocket C/S Connect
#[derive(Clone, Debug)]
pub struct WebSocketConnection {
    /// URL
    url: String,
    /// Local node identity (hex-encoded protobuf ActrId bytes), sent as X-Actr-Source-ID in handshake request for direct-connect mode
    local_id_hex: Option<String>,
    /// Local AIdCredential (base64-encoded), sent with X-Actr-Credential header during handshake for peer verification
    credential_b64: Option<String>,
    /// Write end (Sink) - using Option to avoid initialization issues
    sink: WsSink,

    /// message route by table: PayloadType -> Sender (using array index, 5 fixed elements, Bytes zero-copy)
    router: Arc<RwLock<[Option<mpsc::Sender<bytes::Bytes>>; 5]>>,

    /// Lane Cache: PayloadType -> Lane (using array index, 5 fixed elements)
    lane_cache: LaneCache<5>,

    /// connection status
    connected: Arc<RwLock<bool>>,
}

impl WebSocketConnection {
    /// Connectto WebSocket service device
    ///
    /// # Arguments
    /// - `url`: WebSocket URL (ws:// or wss://)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let conn = WebSocketConnection::new("ws://localhost:8080");
    /// conn.connect().await?;
    /// ```
    pub fn new(url: String) -> Self {
        Self {
            url: url.clone(),
            local_id_hex: None,
            credential_b64: None,
            sink: Arc::new(Mutex::new(None)), // initial begin as None
            router: Arc::new(RwLock::new([None, None, None, None, None])),
            lane_cache: Arc::new(RwLock::new([None, None, None, None, None])),
            connected: Arc::new(RwLock::new(false)),
        }
    }

    /// Set local node identity; automatically appends X-Actr-Source-ID header during handshake (used in direct-connect mode)
    pub fn with_local_id(mut self, id_hex: String) -> Self {
        self.local_id_hex = Some(id_hex);
        self
    }

    /// Set local node AIdCredential (base64-encoded), sent with X-Actr-Credential header during handshake
    pub fn with_credential_b64(mut self, credential_b64: String) -> Self {
        self.credential_b64 = Some(credential_b64);
        self
    }

    /// Create connection from a server-side WebSocket stream with completed handshake (used for direct-connect inbound)
    ///
    /// Unlike `new()` + `connect()`, this method is for already-accepted server connections
    /// where the handshake has been completed by `WebSocketServer`, entering Ready state directly.
    ///
    /// `server.rs` uses `accept_hdr_async(MaybeTlsStream::Plain(stream), ...)` to produce
    /// `WebSocketStream<MaybeTlsStream<TcpStream>>`, identical to the client type, no conversion needed.
    pub fn from_server_stream(ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
        let (sink, stream) = ws_stream.split();

        let router: Arc<RwLock<[Option<mpsc::Sender<bytes::Bytes>>; 5]>> =
            Arc::new(RwLock::new([None, None, None, None, None]));
        let connected = Arc::new(RwLock::new(true));

        Self::spawn_dispatcher(stream, router.clone(), connected.clone());

        tracing::info!("✅ WebSocketConnection created from server stream (already connected)");

        Self {
            url: String::from("<inbound>"),
            local_id_hex: None,
            credential_b64: None,
            sink: Arc::new(Mutex::new(Some(sink))),
            router,
            lane_cache: Arc::new(RwLock::new([None, None, None, None, None])),
            connected,
        }
    }

    /// establish Connect
    pub async fn connect(&self) -> NetworkResult<()> {
        // 1. Establish WebSocket connection (direct-connect mode carries X-Actr-Source-ID header)
        let (ws_stream, _) = if let Some(ref hex_id) = self.local_id_hex {
            // tungstenite does not auto-complete WebSocket upgrade headers; all must be specified manually.
            // Missing any of (Host/Connection/Upgrade/Sec-WebSocket-Version/Sec-WebSocket-Key)
            // will cause the handshake to fail.
            let uri: WsUri = self
                .url
                .parse()
                .map_err(|e| NetworkError::ConnectionError(format!("Invalid WS URI: {e}")))?;
            let host = uri
                .host()
                .ok_or_else(|| NetworkError::ConnectionError("WS URL missing host".to_string()))?;
            let host_header = match uri.port_u16() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            };
            let mut builder = WsRequest::builder()
                .uri(self.url.as_str())
                .header("Host", host_header)
                .header("Connection", "Upgrade")
                .header("Upgrade", "websocket")
                .header("Sec-WebSocket-Version", "13")
                .header("Sec-WebSocket-Key", generate_key())
                .header("X-Actr-Source-ID", hex_id);

            if let Some(ref cred_b64) = self.credential_b64 {
                builder = builder.header("X-Actr-Credential", cred_b64.as_str());
            }

            let request = builder.body(()).map_err(|e| {
                NetworkError::ConnectionError(format!("WS request build failed: {e}"))
            })?;
            connect_async(request).await?
        } else {
            connect_async(&self.url).await?
        };
        let (sink, stream) = ws_stream.split();

        // 2. update new sink
        *self.sink.lock().await = Some(sink);
        *self.connected.write().await = true;

        // 3. Startmessage dispatch device （in background task， not retain handle）
        let router = self.router.clone();
        let connected = self.connected.clone();
        Self::spawn_dispatcher(stream, router, connected);

        tracing::info!("✅ WebSocketConnection already Connect: {}", self.url);

        Ok(())
    }

    /// Checkwhether already Connect
    #[inline]
    pub fn is_connected(&self) -> bool {
        *self.connected.blocking_read()
    }

    /// Startmessage dispatch device （in background task）
    fn spawn_dispatcher(
        mut stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        router: Arc<RwLock<[Option<mpsc::Sender<bytes::Bytes>>; 5]>>,
        connected: Arc<RwLock<bool>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            tracing::debug!("📡 WebSocket dispatcher Start");

            while let Some(msg_result) = stream.next().await {
                match msg_result {
                    Ok(WsMessage::Binary(data)) => {
                        // decodemessage
                        match TransportMessage::decode(&data) {
                            Ok(transport_msg) => {
                                // Route to corresponding 's Lane（using array index reference ）
                                let idx = transport_msg.payload_type as usize;
                                let router_guard = router.read().await;
                                if let Some(tx) = &router_guard[idx] {
                                    // convert exchange as Bytes（ zero-copy）
                                    let data = bytes::Bytes::from(transport_msg.data);
                                    if let Err(e) = tx.send(data).await {
                                        tracing::warn!(
                                            "❌ WebSocket message route by failure (type={:?}): {}",
                                            transport_msg.payload_type,
                                            e
                                        );
                                    }
                                } else {
                                    tracing::warn!(
                                        "⚠️ WebSocket received not RegisterType'smessage: {:?}",
                                        transport_msg.payload_type
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::error!("❌ WebSocket message decodefailure: {}", e);
                            }
                        }
                    }
                    Ok(WsMessage::Close(_)) => {
                        tracing::info!("🔌 WebSocket Connect be pair end Close");
                        *connected.write().await = false;
                        break;
                    }
                    Ok(WsMessage::Ping(_)) | Ok(WsMessage::Pong(_)) => {
                        // ignore center skipmessage
                    }
                    Ok(_) => {
                        tracing::debug!("⚠️ Received non-binary WebSocket message, ignoring");
                    }
                    Err(e) => {
                        tracing::error!("❌ WebSocket Error: {}", e);
                        *connected.write().await = false;
                        break;
                    }
                }
            }

            tracing::debug!("📡 WebSocket dispatcher rollback exit ");
        })
    }

    /// Register PayloadType route by
    async fn register_route(
        &self,
        payload_type: PayloadType,
        tx: mpsc::Sender<bytes::Bytes>,
    ) -> NetworkResult<()> {
        let mut router = self.router.write().await;
        let idx = payload_type as usize;
        router[idx] = Some(tx);
        tracing::debug!("✅ Register WebSocket route by : {:?}", payload_type);
        Ok(())
    }
}

impl WebSocketConnection {
    /// Get or create DataLane (with caching)
    pub async fn get_lane(&self, payload_type: PayloadType) -> NetworkResult<Arc<dyn DataLane>> {
        self.get_lane_internal(payload_type).await
    }

    /// Internal implementation of get_lane
    async fn get_lane_internal(
        &self,
        payload_type: PayloadType,
    ) -> NetworkResult<Arc<dyn DataLane>> {
        let idx = payload_type as usize;

        // 1. Check cache
        {
            let cache = self.lane_cache.read().await;
            if let Some(lane) = &cache[idx] {
                tracing::debug!("Reuse cached DataLane: {:?}", payload_type);
                return Ok(Arc::clone(lane));
            }
        }

        // 2. Create new DataLane
        let lane = self.create_lane_internal(payload_type).await?;

        // 3. Cache
        {
            let mut cache = self.lane_cache.write().await;
            cache[idx] = Some(Arc::clone(&lane));
        }

        tracing::info!(
            "WebSocketConnection created new DataLane: {:?}",
            payload_type
        );

        Ok(lane)
    }

    /// Internal: Create DataLane (without cache)
    async fn create_lane_internal(
        &self,
        payload_type: PayloadType,
    ) -> NetworkResult<Arc<dyn DataLane>> {
        // Check connection status
        if !*self.connected.read().await {
            return Err(NetworkError::ConnectionError(
                "WebSocket connection closed".to_string(),
            ));
        }

        // Create receive channel
        let (tx, rx) = mpsc::channel(100);

        // Register route
        self.register_route(payload_type, tx).await?;

        // Get shared Sink
        let sink = self.sink.clone();

        // Create WebSocketDataLane
        Ok(Arc::new(WebSocketDataLane::new(sink, payload_type, rx)))
    }

    /// Close connection
    pub async fn close(&self) -> NetworkResult<()> {
        *self.connected.write().await = false;

        // Close WebSocket（Send Close message）
        let mut sink_opt = self.sink.lock().await;
        if let Some(sink) = sink_opt.as_mut() {
            let _ = sink.close().await;
        }
        *sink_opt = None;

        // clear blank route by table
        let mut router = self.router.write().await;
        *router = [None, None, None, None, None];

        // clear blank Lane Cache
        let mut cache = self.lane_cache.write().await;
        *cache = [None, None, None, None, None];

        tracing::info!("WebSocketConnection closed");
        Ok(())
    }
}

#[async_trait]
impl WireHandle for WebSocketConnection {
    fn connection_type(&self) -> ConnType {
        ConnType::WebSocket
    }

    fn priority(&self) -> u8 {
        0
    }

    async fn connect(&self) -> NetworkResult<()> {
        self.connect().await
    }

    fn is_connected(&self) -> bool {
        self.is_connected()
    }

    async fn close(&self) -> NetworkResult<()> {
        Self::close(self).await
    }

    async fn get_lane(&self, payload_type: PayloadType) -> NetworkResult<Arc<dyn DataLane>> {
        self.get_lane_internal(payload_type).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transport_message_decode() {
        // Manually construct encoded message:
        // [payload_type: 1 byte][data_len: 4 bytes][data: N bytes]
        let mut encoded = Vec::new();
        encoded.push(PayloadType::RpcReliable as u8); // payload_type = 0
        encoded.extend_from_slice(&11u32.to_be_bytes()); // length = 11
        encoded.extend_from_slice(b"hello world"); // data

        let decoded = TransportMessage::decode(&encoded)
            .expect("Should decode valid TransportMessage in test");

        assert_eq!(decoded.payload_type as u8, PayloadType::RpcReliable as u8);
        assert_eq!(decoded.data, b"hello world");
    }

    #[test]
    fn test_transport_message_decode_invalid() {
        // message too short
        let data = vec![1, 0, 0];
        assert!(TransportMessage::decode(&data).is_err());

        // no effect 's payload_type
        let data = vec![99, 0, 0, 0, 5, 1, 2, 3, 4, 5];
        assert!(TransportMessage::decode(&data).is_err());
    }
}
