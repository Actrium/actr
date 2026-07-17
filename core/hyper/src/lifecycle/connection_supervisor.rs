//! Hierarchical connection-recovery policy.
//!
//! The supervisor deliberately composes independent state machines instead of
//! flattening their Cartesian product into states such as
//! `OfflineForceReconnect`:
//!
//! - `path` owns the latest platform reachability fact and the offline grace;
//! - `recovery` owns the highest-priority recovery intent still to execute;
//! - `offline_work` owns the transport-disconnect work created when an offline
//!   observation is committed.
//!
//! Async execution is a separate concern, modelled by
//! `RecoveryExecutionTracker`. Keeping these layers orthogonal makes a deferred
//! reconnect survive an offline interval without confusing reachability with an
//! action or an execution phase.

use yasm::StateMachine;

use super::network_event::{
    AppLifecycleState, CleanupReason, LONG_BACKGROUND_RECONNECT_THRESHOLD_MS, NetworkEvent,
    NetworkRecoveryAction, NetworkSnapshot, ReconnectReason,
};

mod path {
    use yasm::define_state_machine;

    define_state_machine! {
        name: NetworkPathMachine,
        states: {
            Unknown,
            Online,
            OfflineCandidate,
            Offline
        },
        inputs: {
            ObserveUnknown,
            ObserveOnline,
            ObserveOffline,
            GraceExpired
        },
        initial: Unknown,
        transitions: {
            Unknown + ObserveUnknown => Unknown,
            Unknown + ObserveOnline => Online,
            Unknown + ObserveOffline => OfflineCandidate,
            Unknown + GraceExpired => Unknown,

            Online + ObserveUnknown => Unknown,
            Online + ObserveOnline => Online,
            Online + ObserveOffline => OfflineCandidate,
            Online + GraceExpired => Online,

            OfflineCandidate + ObserveUnknown => Unknown,
            OfflineCandidate + ObserveOnline => Online,
            OfflineCandidate + ObserveOffline => OfflineCandidate,
            OfflineCandidate + GraceExpired => Offline,

            Offline + ObserveUnknown => Unknown,
            Offline + ObserveOnline => Online,
            Offline + ObserveOffline => Offline,
            Offline + GraceExpired => Offline
        }
    }
}

mod recovery {
    use yasm::define_state_machine;

    // Requests are ordered by impact:
    // Probe < Restore < Reconnect < Cleanup.
    // A lower-priority observation cannot downgrade work already requested.
    define_state_machine! {
        name: RecoveryIntentMachine,
        states: {
            Idle,
            ProbePending,
            RestorePending,
            ReconnectPending,
            CleanupPending
        },
        inputs: {
            RequestProbe,
            RequestRestore,
            RequestReconnect,
            RequestCleanup,
            CompleteProbe,
            CompleteRestore,
            CompleteReconnect,
            CompleteCleanup
        },
        initial: Idle,
        transitions: {
            Idle + RequestProbe => ProbePending,
            Idle + RequestRestore => RestorePending,
            Idle + RequestReconnect => ReconnectPending,
            Idle + RequestCleanup => CleanupPending,

            ProbePending + RequestProbe => ProbePending,
            ProbePending + RequestRestore => RestorePending,
            ProbePending + RequestReconnect => ReconnectPending,
            ProbePending + RequestCleanup => CleanupPending,
            ProbePending + CompleteProbe => Idle,

            RestorePending + RequestProbe => RestorePending,
            RestorePending + RequestRestore => RestorePending,
            RestorePending + RequestReconnect => ReconnectPending,
            RestorePending + RequestCleanup => CleanupPending,
            RestorePending + CompleteRestore => Idle,

            ReconnectPending + RequestProbe => ReconnectPending,
            ReconnectPending + RequestRestore => ReconnectPending,
            ReconnectPending + RequestReconnect => ReconnectPending,
            ReconnectPending + RequestCleanup => CleanupPending,
            ReconnectPending + CompleteReconnect => Idle,

            CleanupPending + RequestProbe => CleanupPending,
            CleanupPending + RequestRestore => CleanupPending,
            CleanupPending + RequestReconnect => CleanupPending,
            CleanupPending + RequestCleanup => CleanupPending,
            CleanupPending + CompleteCleanup => Idle
        }
    }
}

mod offline_work {
    use yasm::define_state_machine;

    define_state_machine! {
        name: OfflineWorkMachine,
        states: {
            Idle,
            DisconnectPending
        },
        inputs: {
            RequestDisconnect,
            CompleteDisconnect,
            SupersedeDisconnect
        },
        initial: Idle,
        transitions: {
            Idle + RequestDisconnect => DisconnectPending,
            Idle + CompleteDisconnect => Idle,
            Idle + SupersedeDisconnect => Idle,

            DisconnectPending + RequestDisconnect => DisconnectPending,
            DisconnectPending + CompleteDisconnect => Idle,
            DisconnectPending + SupersedeDisconnect => Idle
        }
    }
}

/// Stable fact model accepted by the connection supervisor.
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

/// Persistent hierarchical policy for mobile/network recovery.
///
/// The latest snapshot sequence is extended state rather than an FSM state
/// because it is an unbounded monotonic value. All finite lifecycle state is
/// owned by one of the three child machines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSupervisor {
    path_state: path::State,
    recovery_state: recovery::State,
    offline_work_state: offline_work::State,
    latest_snapshot_sequence: Option<u64>,
}

impl Default for ConnectionSupervisor {
    fn default() -> Self {
        Self {
            path_state: path::State::Unknown,
            recovery_state: recovery::State::Idle,
            offline_work_state: offline_work::State::Idle,
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

    /// Select one immediate action for an already-collected event slice.
    ///
    /// This compatibility helper has no timer owner, so it commits a final
    /// offline candidate immediately. The long-lived reconciler instead calls
    /// `expire_offline_grace` only after its grace timer fires.
    pub fn select_action(events: &[NetworkEvent]) -> NetworkRecoveryAction {
        let mut supervisor = Self::from_events(events);
        if supervisor.offline_grace_pending() {
            supervisor.expire_offline_grace();
        }
        supervisor.reconcile()
    }

    pub fn submit_event(&mut self, event: &NetworkEvent) {
        self.submit_fact(ConnectionFact::from_network_event(event));
    }

    pub fn submit_fact(&mut self, fact: ConnectionFact) {
        match fact {
            ConnectionFact::CleanupRequested(_) => {
                self.transition_recovery(recovery::Input::RequestCleanup);
            }
            ConnectionFact::ForceReconnectRequested(_) => {
                self.transition_recovery(recovery::Input::RequestReconnect);
            }
            ConnectionFact::NetworkSnapshotChanged(snapshot) => {
                if self
                    .latest_snapshot_sequence
                    .is_some_and(|sequence| snapshot.sequence <= sequence)
                {
                    tracing::debug!(
                        sequence = snapshot.sequence,
                        latest_sequence = ?self.latest_snapshot_sequence,
                        "network_event.supervisor.snapshot_ignored"
                    );
                    return;
                }

                self.latest_snapshot_sequence = Some(snapshot.sequence);
                match snapshot.availability {
                    super::NetworkAvailability::Unknown => {
                        self.transition_path(path::Input::ObserveUnknown);
                        self.transition_offline_work(offline_work::Input::SupersedeDisconnect);
                        self.transition_recovery(recovery::Input::RequestProbe);
                    }
                    super::NetworkAvailability::Available => {
                        self.transition_path(path::Input::ObserveOnline);
                        self.transition_offline_work(offline_work::Input::SupersedeDisconnect);
                        self.transition_recovery(recovery::Input::RequestRestore);
                    }
                    super::NetworkAvailability::Unavailable => {
                        self.transition_path(path::Input::ObserveOffline);
                    }
                }
            }
            ConnectionFact::AppEnteredForeground {
                background_duration_ms,
            } => {
                let input = if background_duration_ms >= LONG_BACKGROUND_RECONNECT_THRESHOLD_MS {
                    recovery::Input::RequestReconnect
                } else {
                    recovery::Input::RequestProbe
                };
                self.transition_recovery(input);
            }
            ConnectionFact::AppEnteredBackground => {}
        }
    }

    /// Whether an unavailable path is still inside the reversible grace phase.
    pub fn offline_grace_pending(&self) -> bool {
        self.path_state == path::State::OfflineCandidate
    }

    /// Commit the latest unavailable observation and request one transport
    /// disconnect. Calling this outside `OfflineCandidate` is idempotent.
    pub fn expire_offline_grace(&mut self) {
        if !self.offline_grace_pending() {
            return;
        }
        self.transition_path(path::Input::GraceExpired);
        self.transition_offline_work(offline_work::Input::RequestDisconnect);
    }

    /// Return the highest-priority action currently allowed by all child
    /// machines. Recovery work remains deferred while the committed path is
    /// offline, but it is not discarded.
    pub fn reconcile(&self) -> NetworkRecoveryAction {
        if self.recovery_state == recovery::State::CleanupPending {
            return NetworkRecoveryAction::CleanupOnly;
        }

        if self.offline_work_state == offline_work::State::DisconnectPending {
            return NetworkRecoveryAction::Offline;
        }

        if matches!(
            self.path_state,
            path::State::OfflineCandidate | path::State::Offline
        ) {
            return NetworkRecoveryAction::Noop;
        }

        match self.recovery_state {
            recovery::State::Idle => NetworkRecoveryAction::Noop,
            recovery::State::ProbePending => NetworkRecoveryAction::Probe,
            recovery::State::RestorePending => NetworkRecoveryAction::Restore,
            recovery::State::ReconnectPending => NetworkRecoveryAction::ForceReconnect,
            recovery::State::CleanupPending => NetworkRecoveryAction::CleanupOnly,
        }
    }

    /// Acknowledge an executed action.
    ///
    /// Failed work stays pending so a later fact can retry it. Successful
    /// cleanup also satisfies a pending offline disconnect because cleanup is
    /// the stronger transport side effect.
    pub fn complete_action(&mut self, action: NetworkRecoveryAction, succeeded: bool) {
        if !succeeded {
            return;
        }

        match action {
            NetworkRecoveryAction::Noop => {}
            NetworkRecoveryAction::Offline => {
                self.transition_offline_work(offline_work::Input::CompleteDisconnect);
            }
            NetworkRecoveryAction::Probe => {
                self.transition_recovery(recovery::Input::CompleteProbe);
            }
            NetworkRecoveryAction::Restore => {
                self.transition_recovery(recovery::Input::CompleteRestore);
            }
            NetworkRecoveryAction::CleanupOnly => {
                self.transition_recovery(recovery::Input::CompleteCleanup);
                self.transition_offline_work(offline_work::Input::SupersedeDisconnect);
            }
            NetworkRecoveryAction::ForceReconnect => {
                self.transition_recovery(recovery::Input::CompleteReconnect);
            }
        }
    }

    fn transition_path(&mut self, input: path::Input) {
        self.path_state =
            <path::NetworkPathMachine as StateMachine>::next_state(&self.path_state, &input)
                .expect("network path transition table must accept every normalized observation");
    }

    fn transition_recovery(&mut self, input: recovery::Input) {
        self.recovery_state = <recovery::RecoveryIntentMachine as StateMachine>::next_state(
            &self.recovery_state,
            &input,
        )
        .expect("recovery intent transition must match the selected action");
    }

    fn transition_offline_work(&mut self, input: offline_work::Input) {
        self.offline_work_state = <offline_work::OfflineWorkMachine as StateMachine>::next_state(
            &self.offline_work_state,
            &input,
        )
        .expect("offline work transition table must be total");
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
    fn reconnect_intent_survives_a_committed_offline_interval() {
        let mut supervisor = ConnectionSupervisor::new();
        supervisor.submit_fact(ConnectionFact::ForceReconnectRequested(
            ReconnectReason::ManualReconnect,
        ));
        supervisor.submit_fact(ConnectionFact::NetworkSnapshotChanged(snapshot(
            1,
            NetworkAvailability::Unavailable,
        )));

        assert!(supervisor.offline_grace_pending());
        assert_eq!(supervisor.reconcile(), NetworkRecoveryAction::Noop);

        supervisor.expire_offline_grace();
        assert_eq!(supervisor.reconcile(), NetworkRecoveryAction::Offline);
        supervisor.complete_action(NetworkRecoveryAction::Offline, true);
        assert_eq!(supervisor.reconcile(), NetworkRecoveryAction::Noop);

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
    fn duplicate_or_stale_snapshot_cannot_move_path_state() {
        let mut supervisor = ConnectionSupervisor::new();
        supervisor.submit_fact(ConnectionFact::NetworkSnapshotChanged(snapshot(
            2,
            NetworkAvailability::Available,
        )));
        supervisor.submit_fact(ConnectionFact::NetworkSnapshotChanged(snapshot(
            2,
            NetworkAvailability::Unavailable,
        )));
        supervisor.submit_fact(ConnectionFact::NetworkSnapshotChanged(snapshot(
            1,
            NetworkAvailability::Unavailable,
        )));

        assert_eq!(supervisor.reconcile(), NetworkRecoveryAction::Restore);
    }

    #[test]
    fn cleanup_priority_does_not_mutate_network_path_state() {
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
        supervisor.complete_action(NetworkRecoveryAction::CleanupOnly, true);
        assert_eq!(supervisor.reconcile(), NetworkRecoveryAction::Noop);
    }

    #[test]
    fn state_machine_docs_keep_orthogonal_layers_separate() {
        let path_doc = StateMachineDoc::<path::NetworkPathMachine>::generate_mermaid();
        let recovery_doc = StateMachineDoc::<recovery::RecoveryIntentMachine>::generate_mermaid();
        let work_doc = StateMachineDoc::<offline_work::OfflineWorkMachine>::generate_mermaid();

        assert!(path_doc.contains("OfflineCandidate"));
        assert!(!path_doc.contains("Reconnect"));
        assert!(recovery_doc.contains("ReconnectPending"));
        assert!(!recovery_doc.contains("Offline"));
        assert!(work_doc.contains("DisconnectPending"));
    }
}
