//! Connection Event System
//!
//! Unified event broadcasting mechanism for connection state changes.
//! Enables proactive resource cleanup across all transport layers.

use actr_protocol::{ActrId, PayloadType};
use tokio::sync::broadcast;

/// Connection state enumeration
/// Maps to WebRTC RTCPeerConnectionState
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// Initial state
    New,
    /// ICE/DTLS negotiation in progress
    Connecting,
    /// Connection established
    Connected,
    /// ICE connectivity lost (may recover)
    Disconnected,
    /// ICE connectivity failed (offerer should try ICE restart)
    Failed,
    /// Connection closed
    Closed,
}

impl std::fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionState::New => write!(f, "New"),
            ConnectionState::Connecting => write!(f, "Connecting"),
            ConnectionState::Connected => write!(f, "Connected"),
            ConnectionState::Disconnected => write!(f, "Disconnected"),
            ConnectionState::Failed => write!(f, "Failed"),
            ConnectionState::Closed => write!(f, "Closed"),
        }
    }
}

/// Connection events broadcast to all subscribers
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    /// Connection state changed
    StateChanged {
        peer_id: ActrId,
        session_id: u64,
        state: ConnectionState,
    },

    /// DataChannel closed for specific payload type
    DataChannelClosed {
        peer_id: ActrId,
        session_id: u64,
        payload_type: PayloadType,
    },

    /// DataChannel opened for specific payload type
    /// This event is fired when a DataChannel transitions to Open state,
    /// indicating SCTP layer is ready for data transmission
    DataChannelOpened {
        peer_id: ActrId,
        session_id: u64,
        payload_type: PayloadType,
    },

    /// Connection fully closed (triggers full cleanup)
    ConnectionClosed { peer_id: ActrId, session_id: u64 },

    /// ICE restart started
    IceRestartStarted { peer_id: ActrId, session_id: u64 },

    /// ICE restart completed
    IceRestartCompleted {
        peer_id: ActrId,
        session_id: u64,
        success: bool,
    },

    /// New offer received (triggers cleanup of existing connection)
    NewOfferReceived { peer_id: ActrId, sdp: String },

    /// New role assignment (triggers cleanup if role changed)
    NewRoleAssignment { peer_id: ActrId, is_offerer: bool },
}

impl ConnectionEvent {
    /// Get the peer_id from the event
    pub fn peer_id(&self) -> &ActrId {
        match self {
            ConnectionEvent::StateChanged { peer_id, .. } => peer_id,
            ConnectionEvent::DataChannelClosed { peer_id, .. } => peer_id,
            ConnectionEvent::DataChannelOpened { peer_id, .. } => peer_id,
            ConnectionEvent::ConnectionClosed { peer_id, .. } => peer_id,
            ConnectionEvent::IceRestartStarted { peer_id, .. } => peer_id,
            ConnectionEvent::IceRestartCompleted { peer_id, .. } => peer_id,
            ConnectionEvent::NewOfferReceived { peer_id, .. } => peer_id,
            ConnectionEvent::NewRoleAssignment { peer_id, .. } => peer_id,
        }
    }

    /// Get the session_id from the event (None for events without session)
    pub fn session_id(&self) -> Option<u64> {
        match self {
            Self::StateChanged { session_id, .. }
            | Self::DataChannelClosed { session_id, .. }
            | Self::DataChannelOpened { session_id, .. }
            | Self::ConnectionClosed { session_id, .. }
            | Self::IceRestartStarted { session_id, .. }
            | Self::IceRestartCompleted { session_id, .. } => Some(*session_id),
            _ => None,
        }
    }

    /// Check if this event should trigger full resource cleanup
    pub fn should_trigger_cleanup(&self) -> bool {
        matches!(
            self,
            ConnectionEvent::ConnectionClosed { .. }
                | ConnectionEvent::StateChanged {
                    state: ConnectionState::Closed,
                    ..
                }
                | ConnectionEvent::IceRestartCompleted { success: false, .. }
        )
    }

    /// Check if this event indicates a recoverable state (ICE restart candidate)
    pub fn is_recoverable_state(&self) -> bool {
        matches!(
            self,
            ConnectionEvent::StateChanged {
                state: ConnectionState::Disconnected | ConnectionState::Failed,
                ..
            }
        )
    }
}

/// Default broadcast channel capacity
const DEFAULT_CHANNEL_CAPACITY: usize = 256;

/// Connection event broadcaster
///
/// Manages a broadcast channel for distributing connection events
/// to all subscribed layers.
#[derive(Debug)]
pub(crate) struct ConnectionEventBroadcaster {
    tx: broadcast::Sender<ConnectionEvent>,
}

impl ConnectionEventBroadcaster {
    /// Create a new broadcaster with default capacity
    pub(crate) fn new() -> Self {
        Self::with_capacity(DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a new broadcaster with specified capacity
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Send an event to all subscribers
    ///
    /// Returns the number of receivers that received the event.
    /// Returns 0 if there are no active subscribers (not an error).
    pub(crate) fn send(&self, event: ConnectionEvent) -> usize {
        self.tx.send(event).unwrap_or_default()
    }

    /// Subscribe to connection events
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<ConnectionEvent> {
        self.tx.subscribe()
    }

    /// Get a clone of the sender for sharing
    pub(crate) fn sender(&self) -> broadcast::Sender<ConnectionEvent> {
        self.tx.clone()
    }
}

impl Default for ConnectionEventBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ConnectionEventBroadcaster {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

#[cfg(test)]
#[path = "connection_event_tests.rs"]
mod tests;
