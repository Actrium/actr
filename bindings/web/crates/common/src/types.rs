//! Common types for Actor-RTC Web

use crate::WebError;
pub use actr_protocol::PayloadType;
use bytes::Bytes;

/// Message format definition.
///
/// All lanes except `MediaTrack` use the same message layout:
/// ```text
/// [PayloadType(1 byte) | Length(4 bytes, Big-Endian) | Data(N bytes)]
/// ```
#[derive(Debug, Clone)]
pub struct MessageFormat {
    pub payload_type: PayloadType,
    pub data: Bytes,
}

impl MessageFormat {
    /// Create a new message.
    pub fn new(payload_type: PayloadType, data: Bytes) -> Self {
        Self { payload_type, data }
    }

    /// Serialize into bytes.
    pub fn serialize(&self) -> Bytes {
        let mut buf = Vec::with_capacity(5 + self.data.len());
        buf.push(self.payload_type as u8);
        buf.extend_from_slice(&(self.data.len() as u32).to_be_bytes());
        buf.extend_from_slice(&self.data);
        Bytes::from(buf)
    }

    /// Alias for `to_bytes`.
    pub fn to_bytes(&self) -> Bytes {
        self.serialize()
    }

    /// Deserialize from bytes.
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.len() < 5 {
            return None;
        }

        let payload_type_byte = data[0];
        let length = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;

        if data.len() < 5 + length {
            return None;
        }

        // Try to convert the payload type.
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

    /// Return the total message length including the header.
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
        // Data is too short.
        let data = vec![0u8, 0, 0, 0];
        assert!(MessageFormat::deserialize(&data).is_none());

        // Length mismatch.
        let data = vec![0u8, 0, 0, 0, 10, 1, 2, 3]; // Claims 10 bytes, actually only 3.
        assert!(MessageFormat::deserialize(&data).is_none());
    }
}
