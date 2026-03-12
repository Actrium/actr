//! Wire Handle - Trait-based abstraction for Wire layer connections
//!
//! WireHandle trait provides a unified interface for different wire connection
//! types (WebRTC, WebSocket, etc.). Platform-specific implementations provide
//! the concrete connection behavior.

use super::error::NetworkResult;
use super::lane::DataLane;
use super::wire_pool::ConnType;
use actr_protocol::PayloadType;
use async_trait::async_trait;
use std::sync::Arc;

/// WireHandle - Unified interface for Wire layer connections
///
/// # Design Philosophy
/// - Uses trait objects for cross-platform extensibility
/// - Supports connection priority comparison (WebRTC > WebSocket)
/// - Platform-specific implementations (native, web) implement this trait
#[async_trait]
pub trait WireHandle: Send + Sync + std::fmt::Debug {
    /// Get connection type
    fn connection_type(&self) -> ConnType;

    /// Connection priority (higher number = higher priority)
    fn priority(&self) -> u8;

    /// Establish connection
    async fn connect(&self) -> NetworkResult<()>;

    /// Check if connected
    fn is_connected(&self) -> bool;

    /// Close connection
    async fn close(&self) -> NetworkResult<()>;

    /// Get or create DataLane (with caching)
    async fn get_lane(&self, payload_type: PayloadType) -> NetworkResult<Arc<dyn DataLane>>;

    /// Invalidate cached lane (no-op by default).
    ///
    /// Used when the underlying transport (e.g. DataChannel) has closed
    /// and needs to be recreated on next `get_lane` call.
    async fn invalidate_lane(&self, _payload_type: PayloadType) {}
}

/// Wire connection status
#[derive(Debug)]
pub enum WireStatus {
    /// Connecting
    Connecting,

    /// Connection ready
    Ready(Arc<dyn WireHandle>),

    /// Connection failed
    Failed,
}

impl Clone for WireStatus {
    fn clone(&self) -> Self {
        match self {
            WireStatus::Connecting => WireStatus::Connecting,
            WireStatus::Ready(handle) => WireStatus::Ready(Arc::clone(handle)),
            WireStatus::Failed => WireStatus::Failed,
        }
    }
}
