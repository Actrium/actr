//! PayloadType option extractor
//!
//! Extracts `option (actr.payload_type)` from protobuf method descriptors.

use prost_types::MethodDescriptorProto;

/// PayloadType enum values matching proto definition
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PayloadType {
    #[default]
    RpcReliable = 0,
    RpcSignal = 1,
    StreamReliable = 2,
    StreamLatencyFirst = 3,
    MediaRtp = 4,
}

impl PayloadType {
    /// Convert from i32 proto enum value
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(PayloadType::RpcReliable),
            1 => Some(PayloadType::RpcSignal),
            2 => Some(PayloadType::StreamReliable),
            3 => Some(PayloadType::StreamLatencyFirst),
            4 => Some(PayloadType::MediaRtp),
            _ => None,
        }
    }

    /// Get Rust enum variant name for code generation
    pub fn as_rust_variant(&self) -> &'static str {
        match self {
            PayloadType::RpcReliable => "PayloadType::RpcReliable",
            PayloadType::RpcSignal => "PayloadType::RpcSignal",
            PayloadType::StreamReliable => "PayloadType::StreamReliable",
            PayloadType::StreamLatencyFirst => "PayloadType::StreamLatencyFirst",
            PayloadType::MediaRtp => "PayloadType::MediaRtp",
        }
    }
}

/// Extract PayloadType option from method descriptor
///
/// # Proto Option Format
///
/// ```protobuf
/// extend google.protobuf.MethodOptions {
///     optional PayloadType payload_type = 50001;
/// }
///
/// rpc SendUrgentMessage(Request) returns (Response) {
///     option (actr.payload_type) = RPC_SIGNAL;
/// }
/// ```
///
/// # Fallback Strategy
///
/// 1. Check method-level option `(actr.payload_type)`
/// 2. If not set, return default: `PayloadType::RpcReliable`
///
/// # Returns
///
/// - `Some(PayloadType)`: Option was explicitly set
/// - `None`: Option not set, should use default
pub fn extract_payload_type(method: &MethodDescriptorProto) -> Option<PayloadType> {
    // The extension field number we defined: 50001
    const _PAYLOAD_TYPE_FIELD_NUMBER: i32 = 50001;

    let _options = method.options.as_ref()?;

    // prost encodes extensions as UnknownFields
    // We need to search through unknown fields for field number 50001
    // However, prost_types::MethodOptions doesn't expose unknown fields directly
    //
    // Alternative approach: Use descriptor.proto's extension mechanism
    // For now, we'll implement a basic parser

    // Check if there's an extension field with our field number
    // This is a simplified implementation - in production you'd want to use
    // protobuf reflection or parse the raw bytes

    // For now, return None to indicate "use default"
    // TODO: Implement proper extension field parsing when prost provides the API
    // or when we have access to FileDescriptorSet

    None
}

/// Check if method uses streaming
fn is_streaming(method: &MethodDescriptorProto) -> bool {
    // Check if input or output is streaming
    method.client_streaming.unwrap_or(false) || method.server_streaming.unwrap_or(false)
}

/// Extract PayloadType with fallback to default
///
/// This is the main API that code generators should use.
/// It always returns a valid PayloadType (never None).
///
/// # Default Rules
///
/// - Unary RPC (no stream) → `RPC_RELIABLE`
/// - Streaming RPC (has stream keyword) → `STREAM_RELIABLE`
/// - Explicit option → uses specified PayloadType
///
/// # Example
///
/// ```rust,ignore
/// let payload_type = extract_payload_type_or_default(method);
/// // Unary RPC: PayloadType::RpcReliable
/// // Streaming RPC: PayloadType::StreamReliable
/// ```
pub fn extract_payload_type_or_default(method: &MethodDescriptorProto) -> PayloadType {
    // 1. Check if explicitly set via option
    if let Some(payload_type) = extract_payload_type(method) {
        return payload_type;
    }

    // 2. Check if streaming - default to STREAM_RELIABLE
    if is_streaming(method) {
        return PayloadType::StreamReliable;
    }

    // 3. Unary RPC - default to RPC_RELIABLE
    PayloadType::RpcReliable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payload_type_from_i32() {
        assert_eq!(PayloadType::from_i32(0), Some(PayloadType::RpcReliable));
        assert_eq!(PayloadType::from_i32(1), Some(PayloadType::RpcSignal));
        assert_eq!(PayloadType::from_i32(2), Some(PayloadType::StreamReliable));
        assert_eq!(
            PayloadType::from_i32(3),
            Some(PayloadType::StreamLatencyFirst)
        );
        assert_eq!(PayloadType::from_i32(4), Some(PayloadType::MediaRtp));
        assert_eq!(PayloadType::from_i32(999), None);
    }

    #[test]
    fn test_payload_type_as_rust_variant() {
        assert_eq!(
            PayloadType::RpcReliable.as_rust_variant(),
            "PayloadType::RpcReliable"
        );
        assert_eq!(
            PayloadType::RpcSignal.as_rust_variant(),
            "PayloadType::RpcSignal"
        );
        assert_eq!(
            PayloadType::StreamReliable.as_rust_variant(),
            "PayloadType::StreamReliable"
        );
        assert_eq!(
            PayloadType::StreamLatencyFirst.as_rust_variant(),
            "PayloadType::StreamLatencyFirst"
        );
        assert_eq!(
            PayloadType::MediaRtp.as_rust_variant(),
            "PayloadType::MediaRtp"
        );
    }

    #[test]
    fn test_default_payload_type() {
        assert_eq!(PayloadType::default(), PayloadType::RpcReliable);
    }

    #[test]
    fn test_extract_unary_rpc_without_options() {
        // Unary RPC: no streaming flags
        let method = MethodDescriptorProto {
            name: Some("TestMethod".to_string()),
            input_type: Some(".test.Request".to_string()),
            output_type: Some(".test.Response".to_string()),
            options: None,
            client_streaming: Some(false),
            server_streaming: Some(false),
        };

        assert_eq!(extract_payload_type(&method), None);
        assert_eq!(
            extract_payload_type_or_default(&method),
            PayloadType::RpcReliable
        );
    }

    #[test]
    fn test_extract_client_streaming_without_options() {
        // Client streaming: stream Request
        let method = MethodDescriptorProto {
            name: Some("UploadFile".to_string()),
            input_type: Some(".test.FileChunk".to_string()),
            output_type: Some(".test.UploadResponse".to_string()),
            options: None,
            client_streaming: Some(true),
            server_streaming: Some(false),
        };

        assert_eq!(
            extract_payload_type_or_default(&method),
            PayloadType::StreamReliable
        );
    }

    #[test]
    fn test_extract_server_streaming_without_options() {
        // Server streaming: returns stream Response
        let method = MethodDescriptorProto {
            name: Some("DownloadFile".to_string()),
            input_type: Some(".test.DownloadRequest".to_string()),
            output_type: Some(".test.FileChunk".to_string()),
            options: None,
            client_streaming: Some(false),
            server_streaming: Some(true),
        };

        assert_eq!(
            extract_payload_type_or_default(&method),
            PayloadType::StreamReliable
        );
    }

    #[test]
    fn test_extract_bidirectional_streaming_without_options() {
        // Bidirectional streaming: stream Request, returns stream Response
        let method = MethodDescriptorProto {
            name: Some("StreamChat".to_string()),
            input_type: Some(".test.ChatMessage".to_string()),
            output_type: Some(".test.ChatMessage".to_string()),
            options: None,
            client_streaming: Some(true),
            server_streaming: Some(true),
        };

        assert_eq!(
            extract_payload_type_or_default(&method),
            PayloadType::StreamReliable
        );
    }

    #[test]
    fn test_is_streaming_detection() {
        // Unary
        let unary = MethodDescriptorProto {
            client_streaming: Some(false),
            server_streaming: Some(false),
            ..Default::default()
        };
        assert!(!is_streaming(&unary));

        // Client streaming
        let client_stream = MethodDescriptorProto {
            client_streaming: Some(true),
            server_streaming: Some(false),
            ..Default::default()
        };
        assert!(is_streaming(&client_stream));

        // Server streaming
        let server_stream = MethodDescriptorProto {
            client_streaming: Some(false),
            server_streaming: Some(true),
            ..Default::default()
        };
        assert!(is_streaming(&server_stream));

        // Bidirectional
        let bidi = MethodDescriptorProto {
            client_streaming: Some(true),
            server_streaming: Some(true),
            ..Default::default()
        };
        assert!(is_streaming(&bidi));
    }
}
