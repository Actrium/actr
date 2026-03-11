//! Wire protocol utilities for Actor-RTC Web
//!
//! Provides helpers for message encoding/decoding and serialization/deserialization.

use crate::error::{WebError, WebResult};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// Serialize a message into JSON bytes.
pub fn serialize_json<T: Serialize>(value: &T) -> WebResult<Bytes> {
    let json = serde_json::to_vec(value).map_err(|e| WebError::Serialization(e.to_string()))?;
    Ok(Bytes::from(json))
}

/// Deserialize a message from JSON bytes.
pub fn deserialize_json<T: for<'de> Deserialize<'de>>(data: &[u8]) -> WebResult<T> {
    serde_json::from_slice(data).map_err(|e| WebError::Serialization(e.to_string()))
}

/// Convert a byte slice into `Bytes`.
pub fn bytes_from_slice(data: &[u8]) -> Bytes {
    Bytes::copy_from_slice(data)
}

/// Convert `Vec<u8>` into `Bytes`.
pub fn bytes_from_vec(data: Vec<u8>) -> Bytes {
    Bytes::from(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct TestMessage {
        id: u32,
        content: String,
    }

    #[test]
    fn test_serialize_deserialize_json() {
        let msg = TestMessage {
            id: 42,
            content: "hello".to_string(),
        };

        let serialized = serialize_json(&msg).unwrap();
        let deserialized: TestMessage = deserialize_json(&serialized).unwrap();

        assert_eq!(deserialized, msg);
    }
}
