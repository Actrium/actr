//! Transport Layer - Service Worker Environment
//!
//! Service Worker 端的传输层实现：
//! - WebSocket Lane：与服务器的 WebSocket 连接
//! - PostMessage Lane：与 DOM 的通信通道
//! - RouteTable：路由表管理

pub mod dest_transport;
pub mod lane;
pub mod peer_transport;
pub mod postmessage;
pub mod route_table;
pub mod sw_transport;
pub mod websocket;
pub mod websocket_connection;
pub mod wire_builder;
pub mod wire_handle;
pub mod wire_pool;

pub use dest_transport::DestTransport;
pub use lane::{DataLane, LaneResult, PortFailureNotifier};
pub use peer_transport::PeerTransport;
pub use postmessage::PostMessageLaneBuilder;
pub use route_table::{RouteEntry, RouteTable, RouteTableStats};
pub use sw_transport::SwTransport;
pub use websocket::WebSocketLaneBuilder;
pub use websocket_connection::WebSocketConnection;
pub use wire_builder::{WebWireBuilder, WireBuilder};
pub use wire_handle::{WebRtcConnection, WireHandle, WireStatus};
pub use wire_pool::{ReadySet, ReadyWatcher, WirePool};
