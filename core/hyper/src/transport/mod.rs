//! Transport Layer 1: Transport layer
//!
//! Core Lane abstraction and transport management:
//! - Lane: Physical embodiment of PayloadType, unified bidirectional channel abstraction
//! - HostTransport: Intra-process transport management (Workload <-> Shell)
//! - PeerTransport: Cross-process transport management (WebRTC + WebSocket)
//! - WireHandle: Unified handle for Wire layer components
//! - WirePool: Wire connection pool manager (strategy layer)
//! - WireBuilder: Wire layer component builder

mod backoff;
pub mod connection_event;
mod dest_transport;
pub mod error;
mod host_transport;
mod lane;
mod peer_transport;
mod route_table;
mod wire_builder;
mod wire_handle;
mod wire_pool;

// Re-export Dest from actr-framework (unified API layer)
pub use actr_framework::Dest;

// DataLane core abstraction (trait + concrete types)
pub use lane::{DataLane, MpscLane, WebRtcDataLane, WebSocketDataLane, WsSink};
pub use route_table::{DataChannelQoS, DataLaneType, PayloadTypeExt, RetryPolicy};

// Transport management
pub use host_transport::HostTransport;
pub use peer_transport::{PeerTransport, WireBuilder};

pub use dest_transport::DestTransport;

// Wire layer management
pub use wire_builder::{DefaultWireBuilder, DefaultWireBuilderConfig};
pub use wire_handle::WireHandle;
pub use wire_pool::{ConnType, WirePool};

// Error types
pub use error::{NetworkError, NetworkResult};

// Retry and backoff strategies
pub use backoff::ExponentialBackoff;

// Connection events
pub use connection_event::{ConnectionEvent, ConnectionEventBroadcaster, ConnectionState};

// Connection session
pub mod session;
pub use session::ConnectionSession;
