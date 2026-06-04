use super::network_event::{
    CleanupReason, LONG_BACKGROUND_RECONNECT_THRESHOLD_MS, NetworkEvent, NetworkRecoveryAction,
    NetworkSnapshot, ReconnectReason,
};

/// Stable fact model used to converge mobile/network lifecycle events before execution.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConnectionFact {
    NetworkOnline,
    NetworkOffline,
    NetworkPathChanged,
    NetworkSnapshotChanged(NetworkSnapshot),
    AppEnteredBackground {
        timestamp_ms: u64,
    },
    AppEnteredForeground {
        background_duration_ms: u64,
        timestamp_ms: u64,
    },
    AppBecameInactive {
        timestamp_ms: u64,
    },
    AppBecameActive {
        timestamp_ms: u64,
    },
    CleanupRequested(CleanupReason),
    ForceReconnectRequested(ReconnectReason),
    ProbeRequested,
}

impl ConnectionFact {
    pub fn from_network_event(event: &NetworkEvent) -> Self {
        match event {
            NetworkEvent::Available => Self::NetworkOnline,
            NetworkEvent::Lost => Self::NetworkOffline,
            NetworkEvent::TypeChanged { .. } => Self::NetworkPathChanged,
            NetworkEvent::PathChanged { snapshot } => {
                Self::NetworkSnapshotChanged(snapshot.clone())
            }
            NetworkEvent::AppEnteredBackground { timestamp_ms } => Self::AppEnteredBackground {
                timestamp_ms: *timestamp_ms,
            },
            NetworkEvent::AppEnteredForeground {
                background_duration_ms,
                timestamp_ms,
            } => Self::AppEnteredForeground {
                background_duration_ms: *background_duration_ms,
                timestamp_ms: *timestamp_ms,
            },
            NetworkEvent::AppBecameInactive { timestamp_ms } => Self::AppBecameInactive {
                timestamp_ms: *timestamp_ms,
            },
            NetworkEvent::AppBecameActive { timestamp_ms } => Self::AppBecameActive {
                timestamp_ms: *timestamp_ms,
            },
            NetworkEvent::AppTerminating { .. } => {
                Self::CleanupRequested(CleanupReason::AppTerminating)
            }
            NetworkEvent::CleanupConnections { reason } => Self::CleanupRequested(*reason),
            NetworkEvent::ForceReconnect { reason } => Self::ForceReconnectRequested(*reason),
            NetworkEvent::ProbeConnectivity => Self::ProbeRequested,
        }
    }
}

/// Pure decision state for a settled connection event batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSupervisor {
    cleanup_requested: Option<CleanupReason>,
    force_reconnect_requested: Option<ReconnectReason>,
    latest_state_action: NetworkRecoveryAction,
    latest_snapshot_timestamp: Option<u64>,
}

impl Default for ConnectionSupervisor {
    fn default() -> Self {
        Self {
            cleanup_requested: None,
            force_reconnect_requested: None,
            latest_state_action: NetworkRecoveryAction::Noop,
            latest_snapshot_timestamp: None,
        }
    }
}

impl ConnectionSupervisor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_events(events: &[NetworkEvent]) -> Self {
        let mut supervisor = Self::new();
        for event in events {
            supervisor.submit_event(event);
        }
        supervisor
    }

    pub fn select_action(events: &[NetworkEvent]) -> NetworkRecoveryAction {
        Self::from_events(events).reconcile()
    }

    pub fn submit_event(&mut self, event: &NetworkEvent) {
        self.submit_fact(ConnectionFact::from_network_event(event));
    }

    pub fn submit_fact(&mut self, fact: ConnectionFact) {
        match fact {
            ConnectionFact::CleanupRequested(reason) => {
                self.cleanup_requested = Some(reason);
            }
            ConnectionFact::ForceReconnectRequested(reason) => {
                self.force_reconnect_requested = Some(reason);
            }
            ConnectionFact::NetworkOnline | ConnectionFact::NetworkPathChanged => {
                self.latest_state_action = NetworkRecoveryAction::Restore;
            }
            ConnectionFact::NetworkOffline => {
                self.latest_state_action = NetworkRecoveryAction::Offline;
            }
            ConnectionFact::NetworkSnapshotChanged(snapshot) => {
                let is_latest = self
                    .latest_snapshot_timestamp
                    .map(|timestamp| snapshot.timestamp_ms >= timestamp)
                    .unwrap_or(true);
                if is_latest {
                    self.latest_snapshot_timestamp = Some(snapshot.timestamp_ms);
                    self.latest_state_action = if snapshot.is_available {
                        NetworkRecoveryAction::Restore
                    } else {
                        NetworkRecoveryAction::Offline
                    };
                }
            }
            ConnectionFact::AppEnteredForeground {
                background_duration_ms,
                ..
            } => {
                if background_duration_ms >= LONG_BACKGROUND_RECONNECT_THRESHOLD_MS {
                    self.force_reconnect_requested = Some(ReconnectReason::LongBackground);
                } else if self.latest_state_action == NetworkRecoveryAction::Noop {
                    self.latest_state_action = NetworkRecoveryAction::Probe;
                }
            }
            ConnectionFact::ProbeRequested => {
                if self.latest_state_action == NetworkRecoveryAction::Noop {
                    self.latest_state_action = NetworkRecoveryAction::Probe;
                }
            }
            ConnectionFact::AppEnteredBackground { .. }
            | ConnectionFact::AppBecameInactive { .. }
            | ConnectionFact::AppBecameActive { .. } => {}
        }
    }

    pub fn reconcile(&self) -> NetworkRecoveryAction {
        if self.cleanup_requested.is_some() {
            NetworkRecoveryAction::CleanupOnly
        } else if self.latest_state_action == NetworkRecoveryAction::Offline {
            NetworkRecoveryAction::Offline
        } else if self.force_reconnect_requested.is_some() {
            NetworkRecoveryAction::ForceReconnect
        } else {
            self.latest_state_action
        }
    }
}
