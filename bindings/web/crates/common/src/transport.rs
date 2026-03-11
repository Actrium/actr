//! Transport abstraction for Web environment
//!
//! Provides a unified transport interface and hides SW/DOM coordination details.

use crate::{PayloadType, WebError, WebResult};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// Destination address.
///
/// It can be:
/// - A peer ID for P2P transport
/// - A server URL for WebSocket transport
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Dest {
    /// P2P peer.
    Peer(String),

    /// Server address.
    Server(String),
}

impl Dest {
    /// Convert the destination into a WebSocket URL.
    pub fn to_websocket_url(&self) -> WebResult<String> {
        match self {
            Dest::Server(url) => Ok(url.clone()),
            Dest::Peer(_) => Err(WebError::Transport(
                "Cannot convert Peer to WebSocket URL".to_string(),
            )),
        }
    }

    /// Return the peer ID, if this is a peer destination.
    pub fn peer_id(&self) -> Option<&str> {
        match self {
            Dest::Peer(id) => Some(id),
            Dest::Server(_) => None,
        }
    }
}

/// Connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not connected.
    Disconnected,
    /// Connecting.
    Connecting,
    /// Connected.
    Connected,
    /// Connection failed.
    Failed,
}

/// Transport statistics.
#[derive(Debug, Clone, Default)]
pub struct TransportStats {
    /// Number of bytes sent.
    pub bytes_sent: u64,
    /// Number of bytes received.
    pub bytes_received: u64,
    /// Number of messages sent.
    pub messages_sent: u64,
    /// Number of messages received.
    pub messages_received: u64,
    /// Number of reconnection attempts.
    pub reconnect_count: u32,
}

/// Connection strategy.
#[derive(Debug, Clone)]
pub struct ConnectionStrategy {
    /// Whether to try multiple connection methods concurrently.
    pub concurrent_attempts: bool,

    /// P2P priority. Larger values mean higher priority.
    pub p2p_priority: u8,

    /// WebSocket priority.
    pub websocket_priority: u8,

    /// Maximum number of retries.
    pub max_retries: u32,

    /// Initial retry delay in milliseconds.
    pub initial_retry_delay_ms: u64,

    /// Maximum retry delay in milliseconds.
    pub max_retry_delay_ms: u64,

    /// Connection timeout in milliseconds.
    pub connection_timeout_ms: u64,
}

impl Default for ConnectionStrategy {
    fn default() -> Self {
        Self {
            concurrent_attempts: true, // Try transports concurrently by default.
            p2p_priority: 100,         // Prefer P2P transport.
            websocket_priority: 50,    // Use WebSocket as fallback.
            max_retries: 5,
            initial_retry_delay_ms: 1000,
            max_retry_delay_ms: 30000,
            connection_timeout_ms: 10000,
        }
    }
}

/// Forwarded message format used for internal SW <-> DOM communication.
///
/// Format: `[Dest(serialized) | PayloadType(1) | Data(N)]`
#[derive(Debug, Clone)]
pub struct ForwardMessage {
    pub dest: Dest,
    pub payload_type: PayloadType,
    pub data: Bytes,
}

impl ForwardMessage {
    /// Create a forwarded message.
    pub fn new(dest: Dest, payload_type: PayloadType, data: Bytes) -> Self {
        Self {
            dest,
            payload_type,
            data,
        }
    }

    /// Serialize into bytes.
    pub fn serialize(&self) -> WebResult<Bytes> {
        // [Dest JSON length(4) | Dest JSON(N) | PayloadType(1) | Data(M)]
        let dest_json = serde_json::to_vec(&self.dest)?;
        let dest_len = dest_json.len() as u32;

        let mut buf = Vec::with_capacity(4 + dest_json.len() + 1 + self.data.len());
        buf.extend_from_slice(&dest_len.to_be_bytes());
        buf.extend_from_slice(&dest_json);
        buf.push(self.payload_type as u8);
        buf.extend_from_slice(&self.data);

        Ok(Bytes::from(buf))
    }

    /// Deserialize from bytes.
    pub fn deserialize(data: &[u8]) -> WebResult<Self> {
        if data.len() < 5 {
            return Err(WebError::Protocol("ForwardMessage too short".to_string()));
        }

        let dest_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if data.len() < 4 + dest_len + 1 {
            return Err(WebError::Protocol("ForwardMessage incomplete".to_string()));
        }

        let dest_json = &data[4..4 + dest_len];
        let dest: Dest = serde_json::from_slice(dest_json)?;

        let payload_type_byte = data[4 + dest_len];
        let payload_type = match payload_type_byte {
            0 => PayloadType::RpcReliable,
            1 => PayloadType::RpcSignal,
            2 => PayloadType::StreamReliable,
            3 => PayloadType::StreamLatencyFirst,
            4 => PayloadType::MediaRtp,
            _ => {
                return Err(WebError::Protocol(format!(
                    "Invalid PayloadType: {}",
                    payload_type_byte
                )));
            }
        };

        let payload_data = Bytes::copy_from_slice(&data[4 + dest_len + 1..]);

        Ok(Self {
            dest,
            payload_type,
            data: payload_data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forward_message_serde() {
        let msg = ForwardMessage::new(
            Dest::Peer("peer123".to_string()),
            PayloadType::RpcReliable,
            Bytes::from_static(b"hello"),
        );

        let serialized = msg.serialize().unwrap();
        let deserialized = ForwardMessage::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.dest, msg.dest);
        assert_eq!(deserialized.payload_type, msg.payload_type);
        assert_eq!(deserialized.data, msg.data);
    }

    #[test]
    fn test_dest_to_websocket_url() {
        let server_dest = Dest::Server("wss://example.com".to_string());
        assert_eq!(server_dest.to_websocket_url().unwrap(), "wss://example.com");

        let peer_dest = Dest::Peer("peer123".to_string());
        assert!(peer_dest.to_websocket_url().is_err());
    }
}
