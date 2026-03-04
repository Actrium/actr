//! Actor-RTC Web Common Library
//!
//! 共享代码库，被 Service Worker Runtime 和 DOM Runtime 共同使用。
//! 包含公共类型、错误定义、消息协议等。

pub mod backoff;
pub mod error;
pub mod events;
pub mod transport;
pub mod types;
pub mod wire;
pub mod zero_copy;

pub use backoff::ExponentialBackoff;
pub use error::{WebError, WebResult};
pub use events::{
    ConnType, ControlMessage, CreateP2PRequest, ErrorCategory, ErrorContext, ErrorReport,
    ErrorSeverity, P2PReadyEvent,
};
pub use transport::{ConnectionState, ConnectionStrategy, Dest, ForwardMessage, TransportStats};
pub use types::{MessageFormat, PayloadType};
