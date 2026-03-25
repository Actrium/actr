//! Actor-RTC Web Common Library
//!
//! Shared code used by both the Service Worker runtime and the DOM runtime.
//! Includes common types, error definitions, and message protocols.

pub mod ais_client;
pub mod backoff;
pub mod error;
pub mod events;
pub mod transport;
pub mod types;
pub mod wire;
pub mod zero_copy;

pub use ais_client::WebAisClient;
pub use backoff::ExponentialBackoff;
pub use error::{WebError, WebResult};
pub use events::{
    ConnType, ControlMessage, CreateP2PRequest, ErrorCategory, ErrorContext, ErrorReport,
    ErrorSeverity, P2PReadyEvent,
};
pub use transport::{ConnectionState, ConnectionStrategy, Dest, ForwardMessage, TransportStats};
pub use types::{MessageFormat, PayloadType};
