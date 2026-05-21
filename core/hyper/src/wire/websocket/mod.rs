//! WebSocket subsystem
//!
//! WebSocket Connection implementation

pub(crate) mod connection;
pub(crate) mod gate;
pub(crate) mod server;

pub(crate) use connection::WebSocketConnection;
pub(crate) use gate::WebSocketGate;
pub(crate) use server::WebSocketServer;
