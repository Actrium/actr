use yasm::StateMachine;

use super::network_event::{
    AppLifecycleState, CleanupReason, LONG_BACKGROUND_RECONNECT_THRESHOLD_MS, NetworkEvent,
    NetworkRecoveryAction, NetworkSnapshot, ReconnectReason,
};

mod policy {
    use yasm::define_state_machine;

    // `OfflineForceReconnect` is intentionally explicit. It remembers that a
    // forced reconnect was requested while the latest network snapshot remains
    // offline, without hiding that combination in independent boolean fields.
    define_state_machine! {
        name: ConnectionPolicyMachine,
        states: {
            Noop,
            Probe,
            Restore,
            ForceReconnect,
            Offline,
            OfflineForceReconnect,
            Cleanup
        },
        inputs: {
            ObserveProbe,
            ObserveRestore,
            ObserveOffline,
            RequestReconnect,
            RequestCleanup,
            EnterBackground,
            EnterForegroundShort,
            EnterForegroundLong
        },
        initial: Noop,
        transitions: {
            Noop + ObserveProbe => Probe,
            Noop + ObserveRestore => Restore,
            Noop + ObserveOffline => Offline,
            Noop + RequestReconnect => ForceReconnect,
            Noop + RequestCleanup => Cleanup,
            Noop + EnterBackground => Noop,
            Noop + EnterForegroundShort => Probe,
            Noop + EnterForegroundLong => ForceReconnect,

            Probe + ObserveProbe => Probe,
            Probe + ObserveRestore => Restore,
            Probe + ObserveOffline => Offline,
            Probe + RequestReconnect => ForceReconnect,
            Probe + RequestCleanup => Cleanup,
            Probe + EnterBackground => Probe,
            Probe + EnterForegroundShort => Probe,
            Probe + EnterForegroundLong => ForceReconnect,

            Restore + ObserveProbe => Probe,
            Restore + ObserveRestore => Restore,
            Restore + ObserveOffline => Offline,
            Restore + RequestReconnect => ForceReconnect,
            Restore + RequestCleanup => Cleanup,
            Restore + EnterBackground => Restore,
            Restore + EnterForegroundShort => Restore,
            Restore + EnterForegroundLong => ForceReconnect,

            ForceReconnect + ObserveProbe => ForceReconnect,
            ForceReconnect + ObserveRestore => ForceReconnect,
            ForceReconnect + ObserveOffline => OfflineForceReconnect,
            ForceReconnect + RequestReconnect => ForceReconnect,
            ForceReconnect + RequestCleanup => Cleanup,
            ForceReconnect + EnterBackground => ForceReconnect,
            ForceReconnect + EnterForegroundShort => ForceReconnect,
            ForceReconnect + EnterForegroundLong => ForceReconnect,

            Offline + ObserveProbe => Probe,
            Offline + ObserveRestore => Restore,
            Offline + ObserveOffline => Offline,
            Offline + RequestReconnect => OfflineForceReconnect,
            Offline + RequestCleanup => Cleanup,
            Offline + EnterBackground => Offline,
            Offline + EnterForegroundShort => Offline,
            Offline + EnterForegroundLong => OfflineForceReconnect,

            OfflineForceReconnect + ObserveProbe => ForceReconnect,
            OfflineForceReconnect + ObserveRestore => ForceReconnect,
            OfflineForceReconnect + ObserveOffline => OfflineForceReconnect,
            OfflineForceReconnect + RequestReconnect => OfflineForceReconnect,
            OfflineForceReconnect + RequestCleanup => Cleanup,
            OfflineForceReconnect + EnterBackground => OfflineForceReconnect,
            OfflineForceReconnect + EnterForegroundShort => OfflineForceReconnect,
            OfflineForceReconnect + EnterForegroundLong => OfflineForceReconnect,

            Cleanup + ObserveProbe => Cleanup,
            Cleanup + ObserveRestore => Cleanup,
            Cleanup + ObserveOffline => Cleanup,
            Cleanup + RequestReconnect => Cleanup,
            Cleanup + RequestCleanup => Cleanup,
            Cleanup + EnterBackground => Cleanup,
            Cleanup + EnterForegroundShort => Cleanup,
            Cleanup + EnterForegroundLong => Cleanup
        }
    }
}

/// Stable fact model used to converge mobile/network lifecycle events before execution.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConnectionFact {
    NetworkSnapshotChanged(NetworkSnapshot),
    AppEnteredBackground,
    AppEnteredForeground { background_duration_ms: u64 },
    CleanupRequested(CleanupReason),
    ForceReconnectRequested(ReconnectReason),
}

impl ConnectionFact {
    pub fn from_network_event(event: &NetworkEvent) -> Self {
        match event {
            NetworkEvent::NetworkPathChanged { snapshot } => {
                Self::NetworkSnapshotChanged(snapshot.clone())
            }
            NetworkEvent::AppLifecycleChanged { state } => match state {
                AppLifecycleState::Background => Self::AppEnteredBackground,
                AppLifecycleState::Foreground {
                    background_duration_ms,
                } => Self::AppEnteredForeground {
                    background_duration_ms: *background_duration_ms,
                },
            },
            NetworkEvent::CleanupConnections { reason } => Self::CleanupRequested(*reason),
            NetworkEvent::ForceReconnect { reason } => Self::ForceReconnectRequested(*reason),
        }
    }
}

/// Deterministic policy state for one connection-event decision cycle.
///
/// The FSM owns all priority and lifecycle transitions. The only extended
/// context is the latest network snapshot sequence because an unbounded
/// monotonic counter cannot be represented as a finite state. Stale snapshots
/// are rejected before their normalized input reaches the FSM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSupervisor {
    policy_state: policy::State,
    latest_snapshot_sequence: Option<u64>,
}

impl Default for ConnectionSupervisor {
    fn default() -> Self {
        Self {
            policy_state: policy::State::Noop,
            latest_snapshot_sequence: None,
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
            ConnectionFact::CleanupRequested(_) => {
                self.transition(policy::Input::RequestCleanup);
            }
            ConnectionFact::ForceReconnectRequested(_) => {
                self.transition(policy::Input::RequestReconnect);
            }
            ConnectionFact::NetworkSnapshotChanged(snapshot) => {
                let is_latest = self
                    .latest_snapshot_sequence
                    .map(|sequence| snapshot.sequence >= sequence)
                    .unwrap_or(true);
                if is_latest {
                    self.latest_snapshot_sequence = Some(snapshot.sequence);
                    let input = if snapshot.is_offline() {
                        policy::Input::ObserveOffline
                    } else if snapshot.should_restore() {
                        policy::Input::ObserveRestore
                    } else {
                        policy::Input::ObserveProbe
                    };
                    self.transition(input);
                }
            }
            ConnectionFact::AppEnteredForeground {
                background_duration_ms,
            } => {
                let input = if background_duration_ms >= LONG_BACKGROUND_RECONNECT_THRESHOLD_MS {
                    policy::Input::EnterForegroundLong
                } else {
                    policy::Input::EnterForegroundShort
                };
                self.transition(input);
            }
            ConnectionFact::AppEnteredBackground => {
                self.transition(policy::Input::EnterBackground);
            }
        }
    }

    pub fn reconcile(&self) -> NetworkRecoveryAction {
        match self.policy_state {
            policy::State::Noop => NetworkRecoveryAction::Noop,
            policy::State::Probe => NetworkRecoveryAction::Probe,
            policy::State::Restore => NetworkRecoveryAction::Restore,
            policy::State::ForceReconnect => NetworkRecoveryAction::ForceReconnect,
            policy::State::Offline | policy::State::OfflineForceReconnect => {
                NetworkRecoveryAction::Offline
            }
            policy::State::Cleanup => NetworkRecoveryAction::CleanupOnly,
        }
    }

    fn transition(&mut self, input: policy::Input) {
        self.policy_state = <policy::ConnectionPolicyMachine as StateMachine>::next_state(
            &self.policy_state,
            &input,
        )
        .expect("connection policy transition table must accept every normalized fact");
    }
}

#[cfg(test)]
mod tests {
    use yasm::StateMachineDoc;

    use super::*;
    use crate::lifecycle::{NetworkAvailability, NetworkTransportFlags};

    fn snapshot(sequence: u64, availability: NetworkAvailability) -> NetworkSnapshot {
        NetworkSnapshot {
            sequence,
            availability,
            transport: NetworkTransportFlags::default(),
            is_expensive: false,
            is_constrained: false,
        }
    }

    #[test]
    fn forced_reconnect_survives_an_offline_interval() {
        let mut supervisor = ConnectionSupervisor::new();
        supervisor.submit_fact(ConnectionFact::ForceReconnectRequested(
            ReconnectReason::ManualReconnect,
        ));
        supervisor.submit_fact(ConnectionFact::NetworkSnapshotChanged(snapshot(
            1,
            NetworkAvailability::Unavailable,
        )));
        assert_eq!(supervisor.reconcile(), NetworkRecoveryAction::Offline);

        supervisor.submit_fact(ConnectionFact::NetworkSnapshotChanged(snapshot(
            2,
            NetworkAvailability::Available,
        )));
        assert_eq!(
            supervisor.reconcile(),
            NetworkRecoveryAction::ForceReconnect
        );
    }

    #[test]
    fn stale_snapshot_cannot_move_the_policy_machine() {
        let mut supervisor = ConnectionSupervisor::new();
        supervisor.submit_fact(ConnectionFact::NetworkSnapshotChanged(snapshot(
            2,
            NetworkAvailability::Available,
        )));
        supervisor.submit_fact(ConnectionFact::NetworkSnapshotChanged(snapshot(
            1,
            NetworkAvailability::Unavailable,
        )));

        assert_eq!(supervisor.reconcile(), NetworkRecoveryAction::Restore);
    }

    #[test]
    fn cleanup_is_an_absorbing_policy_state() {
        let mut supervisor = ConnectionSupervisor::new();
        supervisor.submit_fact(ConnectionFact::CleanupRequested(
            CleanupReason::AppTerminating,
        ));
        supervisor.submit_fact(ConnectionFact::NetworkSnapshotChanged(snapshot(
            1,
            NetworkAvailability::Available,
        )));
        supervisor.submit_fact(ConnectionFact::ForceReconnectRequested(
            ReconnectReason::ManualReconnect,
        ));

        assert_eq!(supervisor.reconcile(), NetworkRecoveryAction::CleanupOnly);
    }

    #[test]
    fn policy_machine_documentation_exposes_composite_offline_state() {
        let mermaid = StateMachineDoc::<policy::ConnectionPolicyMachine>::generate_mermaid();

        assert!(mermaid.contains("OfflineForceReconnect"));
        assert!(mermaid.contains("RequestCleanup"));
        assert!(mermaid.contains("EnterForegroundLong"));
    }
}
