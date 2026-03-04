//! Wire protocol utilities for Actor-RTC Web
//!
//! 提供消息编码/解码、序列化/反序列化等工具函数

use crate::error::{WebError, WebResult};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// 将消息序列化为 JSON 字节流
pub fn serialize_json<T: Serialize>(value: &T) -> WebResult<Bytes> {
    let json = serde_json::to_vec(value).map_err(|e| WebError::Serialization(e.to_string()))?;
    Ok(Bytes::from(json))
}

/// 从 JSON 字节流反序列化消息
pub fn deserialize_json<T: for<'de> Deserialize<'de>>(data: &[u8]) -> WebResult<T> {
    serde_json::from_slice(data).map_err(|e| WebError::Serialization(e.to_string()))
}

/// 将字节数组转换为 Bytes（零拷贝）
pub fn bytes_from_slice(data: &[u8]) -> Bytes {
    Bytes::copy_from_slice(data)
}

/// 将 Vec<u8> 转换为 Bytes（零拷贝）
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
