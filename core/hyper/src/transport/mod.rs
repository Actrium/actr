//! Transport Layer 1: Transport layer
//!
//! Core Lane abstraction and transport management:
//! - Lane: Physical embodiment of PayloadType, unified bidirectional channel abstraction
//! - HostTransport: Intra-process transport management (Guest <-> Shell)
//! - PeerTransport: Cross-process transport management (WebRTC + WebSocket)
//! - WireHandle: Unified handle for Wire layer components
//! - WirePool: Wire connection pool manager (strategy layer)
//! - WireBuilder: Wire layer component builder

mod backoff;
mod connection_event;
mod dest_transport;
mod error;
mod host_transport;
pub(crate) mod lane;
mod peer_transport;
mod route_table;
mod wire_builder;
mod wire_handle;
pub(crate) mod wire_pool;

// Re-export Dest from actr-framework (unified API layer).
pub use actr_framework::Dest;

// Submodule-internal types (lanes, wire pool, sessions, dest_transport,
// connection events broadcasters) stay reachable via their module paths
// without duplicating re-exports here.

// DataLane core abstraction (trait kept pub for sw-host/peer_transport).
#[cfg(feature = "test-utils")]
pub use lane::DataLane;
#[cfg(not(feature = "test-utils"))]
pub(crate) use lane::DataLane;
pub(crate) use lane::{MpscLane, WebRtcDataLane, WebSocketDataLane, WsSink};
pub(crate) use route_table::PayloadTypeExt;

// ConnType leaks through the public `WireHandle::connection_type` method,
// so it must stay pub even though the `wire_pool` module itself is private.
#[cfg(feature = "test-utils")]
pub use wire_pool::ConnType;
#[cfg(not(feature = "test-utils"))]
pub(crate) use wire_pool::ConnType;

// Transport management
#[cfg(feature = "test-utils")]
pub use host_transport::HostTransport;
#[cfg(not(feature = "test-utils"))]
pub(crate) use host_transport::HostTransport;
#[cfg(not(feature = "test-utils"))]
pub(crate) use peer_transport::PeerTransport;
#[cfg(feature = "test-utils")]
pub use peer_transport::{PeerTransport, WireBuilder};

// Wire layer management
#[cfg(feature = "test-utils")]
pub use wire_builder::{DefaultWireBuilder, DefaultWireBuilderConfig};
#[cfg(not(feature = "test-utils"))]
pub(crate) use wire_builder::{DefaultWireBuilder, DefaultWireBuilderConfig};
#[cfg(feature = "test-utils")]
pub use wire_handle::WireHandle;
#[cfg(not(feature = "test-utils"))]
pub(crate) use wire_handle::WireHandle;

// Error types
pub use error::{NetworkError, NetworkResult};

// Retry and backoff strategies
pub use backoff::ExponentialBackoff;

// Connection events are re-exported at the transport module boundary; the
// broadcaster stays crate-internal.
pub(crate) use connection_event::ConnectionEventBroadcaster;
pub use connection_event::{ConnectionEvent, ConnectionState};

// Connection session
pub(crate) mod session;
