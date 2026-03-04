//! Error types for Actor-RTC Web

use thiserror::Error;

/// Result type for Web operations
pub type WebResult<T> = std::result::Result<T, WebError>;

/// Web error types
#[derive(Error, Debug, Clone)]
pub enum WebError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Timeout error")]
    Timeout,

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Mailbox error: {0}")]
    Mailbox(String),

    #[error("Invalid configuration: {0}")]
    Config(String),

    #[error("Service not found: {0}")]
    ServiceNotFound(String),

    #[error("Method not found: {0}")]
    MethodNotFound(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("JavaScript error: {0}")]
    Js(String),

    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Fast path error: {0}")]
    FastPath(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Channel closed: {0}")]
    ChannelClosed(String),
}

impl From<serde_json::Error> for WebError {
    fn from(error: serde_json::Error) -> Self {
        WebError::Serialization(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_error_network() {
        let error = WebError::Network("Connection refused".to_string());
        assert_eq!(error.to_string(), "Network error: Connection refused");
    }

    #[test]
    fn test_web_error_serialization() {
        let error = WebError::Serialization("Invalid JSON".to_string());
        assert_eq!(error.to_string(), "Serialization error: Invalid JSON");
    }

    #[test]
    fn test_web_error_timeout() {
        let error = WebError::Timeout;
        assert_eq!(error.to_string(), "Timeout error");
    }

    #[test]
    fn test_web_error_connection() {
        let error = WebError::Connection("Handshake failed".to_string());
        assert_eq!(error.to_string(), "Connection error: Handshake failed");
    }

    #[test]
    fn test_web_error_mailbox() {
        let error = WebError::Mailbox("Queue full".to_string());
        assert_eq!(error.to_string(), "Mailbox error: Queue full");
    }

    #[test]
    fn test_web_error_config() {
        let error = WebError::Config("Missing required field".to_string());
        assert_eq!(
            error.to_string(),
            "Invalid configuration: Missing required field"
        );
    }

    #[test]
    fn test_web_error_service_not_found() {
        let error = WebError::ServiceNotFound("UserService".to_string());
        assert_eq!(error.to_string(), "Service not found: UserService");
    }

    #[test]
    fn test_web_error_method_not_found() {
        let error = WebError::MethodNotFound("getUserById".to_string());
        assert_eq!(error.to_string(), "Method not found: getUserById");
    }

    #[test]
    fn test_web_error_internal() {
        let error = WebError::Internal("Unexpected state".to_string());
        assert_eq!(error.to_string(), "Internal error: Unexpected state");
    }

    #[test]
    fn test_web_error_js() {
        let error = WebError::Js("TypeError: undefined is not a function".to_string());
        assert_eq!(
            error.to_string(),
            "JavaScript error: TypeError: undefined is not a function"
        );
    }

    #[test]
    fn test_web_error_transport() {
        let error = WebError::Transport("Send failed".to_string());
        assert_eq!(error.to_string(), "Transport error: Send failed");
    }

    #[test]
    fn test_web_error_fast_path() {
        let error = WebError::FastPath("Handler not found".to_string());
        assert_eq!(error.to_string(), "Fast path error: Handler not found");
    }

    #[test]
    fn test_web_error_protocol() {
        let error = WebError::Protocol("Invalid message format".to_string());
        assert_eq!(error.to_string(), "Protocol error: Invalid message format");
    }

    #[test]
    fn test_web_error_from_serde_json() {
        let json_error = serde_json::from_str::<serde_json::Value>("{invalid json}")
            .expect_err("Should fail to parse");
        let web_error: WebError = json_error.into();

        match web_error {
            WebError::Serialization(msg) => {
                // 只验证错误消息不为空
                assert!(!msg.is_empty());
            }
            _ => panic!("Expected Serialization error"),
        }
    }

    #[test]
    fn test_web_result_ok() {
        let result: WebResult<i32> = Ok(42);
        assert!(result.is_ok());
        if let Ok(value) = result {
            assert_eq!(value, 42);
        }
    }

    #[test]
    fn test_web_result_err() {
        let result: WebResult<i32> = Err(WebError::Timeout);
        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.to_string(), "Timeout error");
        }
    }

    #[test]
    fn test_error_clone() {
        let error1 = WebError::Network("Test".to_string());
        let error2 = error1.clone();

        assert_eq!(error1.to_string(), error2.to_string());
    }

    #[test]
    fn test_error_debug() {
        let error = WebError::Transport("Debug test".to_string());
        let debug_str = format!("{:?}", error);
        assert!(debug_str.contains("Transport"));
        assert!(debug_str.contains("Debug test"));
    }
}
