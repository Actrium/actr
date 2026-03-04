//! Common types for Actor-RTC Web

use crate::WebError;
pub use actr_protocol::PayloadType;
use bytes::Bytes;

/// 消息格式定义
///
/// 所有 Lane（除 MediaTrack）使用统一的消息格式：
/// ```text
/// [PayloadType(1字节) | Length(4字节, Big-Endian) | Data(N字节)]
/// ```
#[derive(Debug, Clone)]
pub struct MessageFormat {
    pub payload_type: PayloadType,
    pub data: Bytes,
}

impl MessageFormat {
    /// 创建新的消息
    pub fn new(payload_type: PayloadType, data: Bytes) -> Self {
        Self { payload_type, data }
    }

    /// 序列化为字节流
    pub fn serialize(&self) -> Bytes {
        let mut buf = Vec::with_capacity(5 + self.data.len());
        buf.push(self.payload_type as u8);
        buf.extend_from_slice(&(self.data.len() as u32).to_be_bytes());
        buf.extend_from_slice(&self.data);
        Bytes::from(buf)
    }

    /// 别名：to_bytes
    pub fn to_bytes(&self) -> Bytes {
        self.serialize()
    }

    /// 从字节流反序列化
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.len() < 5 {
            return None;
        }

        let payload_type_byte = data[0];
        let length = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;

        if data.len() < 5 + length {
            return None;
        }

        // 尝试转换 PayloadType
        let payload_type = match payload_type_byte {
            0 => PayloadType::RpcReliable,
            1 => PayloadType::RpcSignal,
            2 => PayloadType::StreamReliable,
            3 => PayloadType::StreamLatencyFirst,
            4 => PayloadType::MediaRtp,
            _ => return None,
        };

        let payload_data = Bytes::copy_from_slice(&data[5..5 + length]);

        Some(Self {
            payload_type,
            data: payload_data,
        })
    }

    /// 获取消息总长度（包含头部）
    pub fn total_len(&self) -> usize {
        5 + self.data.len()
    }
}

impl TryFrom<Bytes> for MessageFormat {
    type Error = WebError;

    fn try_from(data: Bytes) -> Result<Self, Self::Error> {
        Self::deserialize(&data)
            .ok_or_else(|| WebError::Protocol("Invalid MessageFormat".to_string()))
    }
}

impl TryFrom<&[u8]> for MessageFormat {
    type Error = WebError;

    fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
        Self::deserialize(data)
            .ok_or_else(|| WebError::Protocol("Invalid MessageFormat".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_format_serialize_deserialize() {
        let msg = MessageFormat::new(PayloadType::RpcReliable, Bytes::from_static(b"hello world"));

        let serialized = msg.serialize();
        let deserialized = MessageFormat::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.payload_type, PayloadType::RpcReliable);
        assert_eq!(deserialized.data, Bytes::from_static(b"hello world"));
    }

    #[test]
    fn test_message_format_invalid_data() {
        // 数据太短
        let data = vec![0u8, 0, 0, 0];
        assert!(MessageFormat::deserialize(&data).is_none());

        // 长度不匹配
        let data = vec![0u8, 0, 0, 0, 10, 1, 2, 3]; // 声称 10 字节，实际只有 3 字节
        assert!(MessageFormat::deserialize(&data).is_none());
    }
}
