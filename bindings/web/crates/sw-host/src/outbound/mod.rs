//! Outbound layer for sending messages.
//!
//! Mirrors actr's outbound layer and provides a unified sending interface.
//!
//! # Outbound path
//!
//! ```text
//! Actor ctx.call()/tell()
//!   -> Gate::Peer
//!     -> PeerGate (ActrId->Dest mapping + pending_requests)
//!       -> PeerTransport (Dest->DestTransport mapping)
//!         -> DestTransport (event-driven send loop)
//!           -> WirePool (priority selection: WebRTC > WebSocket)
//!             -> WireHandle::WebRTC.get_lane()
//!               -> DataLane::PostMessage { port: dedicated MessagePort }
//!                 -> port.postMessage(data)  [zero-copy, no command protocol]
//!                   -> DOM bridge -> RtcDataChannel.send() -> Remote
//! ```

mod host_gate;
mod peer_gate;

pub use host_gate::HostGate;
pub use peer_gate::PeerGate;

use actr_protocol::{ActorResult, ActrId, PayloadType, RpcEnvelope};
use bytes::Bytes;
use std::sync::Arc;

/// Gate enum for outbound messaging.
///
/// # Variants
///
/// - **Host**: communication between actors inside the SW with zero serialization
/// - **Peer**: cross-node transport through a dedicated MessagePort and the full transport stack
#[derive(Clone)]
pub enum Gate {
    /// Host gate for in-SW communication with zero serialization.
    Host(Arc<HostGate>),

    /// Peer gate for cross-node transport.
    ///
    /// PeerGate -> PeerTransport -> DestTransport
    ///   -> WirePool -> WireHandle -> DataLane::PostMessage (direct send through a dedicated MessagePort)
    Peer(Arc<PeerGate>),
}

impl Gate {
    /// Create a Host gate.
    pub fn host(gate: Arc<HostGate>) -> Self {
        Self::Host(gate)
    }

    /// Create a Peer gate.
    pub fn peer(gate: Arc<PeerGate>) -> Self {
        Self::Peer(gate)
    }

    /// Send a request and wait for the response.
    pub async fn send_request(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<Bytes> {
        match self {
            Gate::Host(gate) => gate.send_request(target, envelope).await,
            Gate::Peer(gate) => gate.send_request(target, envelope).await,
        }
    }

    /// Send a one-way message without waiting for a response.
    pub async fn send_message(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<()> {
        match self {
            Gate::Host(gate) => gate.send_message(target, envelope).await,
            Gate::Peer(gate) => gate.send_message(target, envelope).await,
        }
    }

    /// Send a DataStream through the Fast Path.
    pub async fn send_data_stream(
        &self,
        target: &ActrId,
        payload_type: PayloadType,
        data: Bytes,
    ) -> ActorResult<()> {
        match self {
            Gate::Host(gate) => gate.send_data_stream(target, payload_type, data).await,
            Gate::Peer(gate) => gate.send_data_stream(target, payload_type, data).await,
        }
    }

    /// Try to handle a remote response.
    ///
    /// Checks whether this gate has a pending request for `request_id`.
    /// If so, resolves it and returns true; otherwise returns false.
    pub fn try_handle_response(&self, request_id: &str, response: Bytes) -> bool {
        match self {
            Gate::Host(_) => false,
            Gate::Peer(gate) => gate.handle_response(request_id.to_string(), response),
        }
    }
}
