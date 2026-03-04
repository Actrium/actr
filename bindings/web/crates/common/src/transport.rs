//! Transport abstraction for Web environment
//!
//! 提供统一的传输层接口，封装 SW 和 DOM 的协作细节

use crate::{PayloadType, WebError, WebResult};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// 目标地址
///
/// 可以是：
/// - Peer ID（P2P 连接）
/// - Server URL（WebSocket 连接）
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Dest {
    /// P2P 对等节点
    Peer(String),

    /// 服务器地址
    Server(String),
}

impl Dest {
    /// 转换为 WebSocket URL
    pub fn to_websocket_url(&self) -> WebResult<String> {
        match self {
            Dest::Server(url) => Ok(url.clone()),
            Dest::Peer(_) => Err(WebError::Transport(
                "Cannot convert Peer to WebSocket URL".to_string(),
            )),
        }
    }

    /// 获取 Peer ID
    pub fn peer_id(&self) -> Option<&str> {
        match self {
            Dest::Peer(id) => Some(id),
            Dest::Server(_) => None,
        }
    }
}

/// 连接状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// 未连接
    Disconnected,
    /// 连接中
    Connecting,
    /// 已连接
    Connected,
    /// 连接失败
    Failed,
}

/// 传输统计信息
#[derive(Debug, Clone, Default)]
pub struct TransportStats {
    /// 发送字节数
    pub bytes_sent: u64,
    /// 接收字节数
    pub bytes_received: u64,
    /// 发送消息数
    pub messages_sent: u64,
    /// 接收消息数
    pub messages_received: u64,
    /// 连接重试次数
    pub reconnect_count: u32,
}

/// 连接策略
#[derive(Debug, Clone)]
pub struct ConnectionStrategy {
    /// 是否并发尝试多种连接方式
    pub concurrent_attempts: bool,

    /// P2P 优先级（值越大优先级越高）
    pub p2p_priority: u8,

    /// WebSocket 优先级
    pub websocket_priority: u8,

    /// 最大重试次数
    pub max_retries: u32,

    /// 初始重试延迟（毫秒）
    pub initial_retry_delay_ms: u64,

    /// 最大重试延迟（毫秒）
    pub max_retry_delay_ms: u64,

    /// 连接超时（毫秒）
    pub connection_timeout_ms: u64,
}

impl Default for ConnectionStrategy {
    fn default() -> Self {
        Self {
            concurrent_attempts: true, // 默认并发尝试
            p2p_priority: 100,         // P2P 优先级高
            websocket_priority: 50,    // WebSocket 作为 fallback
            max_retries: 5,
            initial_retry_delay_ms: 1000,
            max_retry_delay_ms: 30000,
            connection_timeout_ms: 10000,
        }
    }
}

/// 转发消息格式（SW ↔ DOM 内部通信）
///
/// 格式：[Dest(序列化) | PayloadType(1) | Data(N)]
#[derive(Debug, Clone)]
pub struct ForwardMessage {
    pub dest: Dest,
    pub payload_type: PayloadType,
    pub data: Bytes,
}

impl ForwardMessage {
    /// 创建转发消息
    pub fn new(dest: Dest, payload_type: PayloadType, data: Bytes) -> Self {
        Self {
            dest,
            payload_type,
            data,
        }
    }

    /// 序列化为字节流
    pub fn serialize(&self) -> WebResult<Bytes> {
        // [Dest JSON 长度(4) | Dest JSON(N) | PayloadType(1) | Data(M)]
        let dest_json = serde_json::to_vec(&self.dest)?;
        let dest_len = dest_json.len() as u32;

        let mut buf = Vec::with_capacity(4 + dest_json.len() + 1 + self.data.len());
        buf.extend_from_slice(&dest_len.to_be_bytes());
        buf.extend_from_slice(&dest_json);
        buf.push(self.payload_type as u8);
        buf.extend_from_slice(&self.data);

        Ok(Bytes::from(buf))
    }

    /// 从字节流反序列化
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
