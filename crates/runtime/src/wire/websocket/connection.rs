//! WebSocket C/S Connection implementation

use crate::transport::DataLane;
use crate::transport::{NetworkError, NetworkResult};
use actr_protocol::PayloadType;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;
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

/// WebSocket Sink Type distinct name
type WsSink = Arc<Mutex<Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>>>>;

/// WebSocketConnection - WebSocket C/S Connect
#[derive(Clone, Debug)]
pub struct WebSocketConnection {
    /// URL
    url: String,
    /// Write end (Sink) - using Option to avoid initialization issues
    sink: WsSink,

    /// message route by table ：PayloadType → Sender（using array index reference ，5 fixed elements，using Bytes zero-copy）
    router: Arc<RwLock<[Option<mpsc::Sender<bytes::Bytes>>; 5]>>,

    /// Lane Cache：PayloadType → Lane（using array index reference ，5 fixed elements）
    lane_cache: Arc<RwLock<[Option<DataLane>; 5]>>,

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
            sink: Arc::new(Mutex::new(None)), // initial begin as None
            router: Arc::new(RwLock::new([None, None, None, None, None])),
            lane_cache: Arc::new(RwLock::new([None, None, None, None, None])),
            connected: Arc::new(RwLock::new(false)),
        }
    }

    /// establish Connect
    pub async fn connect(&self) -> NetworkResult<()> {
        // 1. establish WebSocket Connect
        let (ws_stream, _) = connect_async(&self.url).await?;
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
    /// GetorCreate DataLane（ carry Cache）
    pub async fn get_lane(&self, payload_type: PayloadType) -> NetworkResult<DataLane> {
        let idx = payload_type as usize;

        // 1. CheckCache
        {
            let cache = self.lane_cache.read().await;
            if let Some(lane) = &cache[idx] {
                tracing::debug!("📦 ReuseCache DataLane: {:?}", payload_type);
                return Ok(lane.clone());
            }
        }

        // 2. Createnew DataLane
        let lane = self.create_lane_internal(payload_type).await?;

        // 3. Cache
        {
            let mut cache = self.lane_cache.write().await;
            cache[idx] = Some(lane.clone());
        }

        tracing::info!(
            "✨ WebSocketConnection Createnew DataLane: {:?}",
            payload_type
        );

        Ok(lane)
    }

    /// inner part Method：Create DataLane（ not carry Cache）
    async fn create_lane_internal(&self, payload_type: PayloadType) -> NetworkResult<DataLane> {
        // Check connection status
        if !*self.connected.read().await {
            return Err(NetworkError::ConnectionError(
                "WebSocket connection closed".to_string(),
            ));
        }

        // CreateReceive channel
        let (tx, rx) = mpsc::channel(100);

        // Register route by
        self.register_route(payload_type, tx).await?;

        // Getshared's Sink
        let sink = self.sink.clone();

        // Create DataLane（usingnew's websocket transform body ）
        Ok(DataLane::websocket(sink, payload_type, rx))
    }

    /// backwardaftercompatible hold Method：create_lane adjust usage get_lane
    pub async fn create_lane(&self, payload_type: PayloadType) -> NetworkResult<DataLane> {
        self.get_lane(payload_type).await
    }

    /// CloseConnect
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

        tracing::info!("🔌 WebSocketConnection already Close");
        Ok(())
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
