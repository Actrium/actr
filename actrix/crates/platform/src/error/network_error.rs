//! 网络相关错误类型
//!
//! 定义所有与网络连接、通信、传输相关的错误

use thiserror::Error;

/// 网络相关错误
#[derive(Error, Debug)]
pub enum NetworkError {
    #[error("Connection failed: {address}")]
    ConnectionFailed { address: String },

    #[error("Connection timeout: {address}")]
    Timeout { address: String },

    #[error("DNS resolution failed: {hostname}")]
    DnsResolution { hostname: String },

    #[error("Invalid address format: {address}")]
    InvalidAddress { address: String },

    #[error("Port binding failed: {port}")]
    PortBindFailed { port: u16 },

    #[error("TLS error: {message}")]
    Tls { message: String },

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("HTTP error: {status}")]
    Http { status: u16 },
}
