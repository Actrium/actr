//! Deterministic execution lifecycle for selected network recovery actions.
//!
//! This FSM owns only the phase of one serialized async operation. Long-lived
//! facts such as online/offline reachability, desired recovery, and the last
//! error belong to other layers and intentionally do not appear here.

use yasm::{StateMachineInstance, define_state_machine};

use super::NetworkRecoveryAction;

define_state_machine! {
    name: RecoveryExecutionMachine,
    states: {
        Idle,
        Disconnecting,
        Probing,
        Restoring,
        Reconnecting,
        Cleaning
    },
    inputs: {
        BeginOffline,
        BeginProbe,
        BeginRestore,
        BeginReconnect,
        BeginCleanup,
        Succeeded,
        Failed
    },
    initial: Idle,
    transitions: {
        Idle + BeginOffline => Disconnecting,
        Idle + BeginProbe => Probing,
        Idle + BeginRestore => Restoring,
        Idle + BeginReconnect => Reconnecting,
        Idle + BeginCleanup => Cleaning,

        Disconnecting + Succeeded => Idle,
        Probing + Succeeded => Idle,
        Restoring + Succeeded => Idle,
        Reconnecting + Succeeded => Idle,
        Cleaning + Succeeded => Idle,

        Disconnecting + Failed => Idle,
        Probing + Failed => Idle,
        Restoring + Failed => Idle,
        Reconnecting + Failed => Idle,
        Cleaning + Failed => Idle
    }
}

pub(crate) struct RecoveryExecutionTracker {
    machine: StateMachineInstance<RecoveryExecutionMachine>,
}

impl Default for RecoveryExecutionTracker {
    fn default() -> Self {
        Self {
            // Recovery history is diagnostic rather than durable. Bound it so
            // a long-running mobile process cannot accumulate unbounded state.
            machine: StateMachineInstance::with_max_history(64),
        }
    }
}

impl RecoveryExecutionTracker {
    pub(crate) fn begin(
        &mut self,
        action: NetworkRecoveryAction,
    ) -> Result<(State, State), String> {
        let input = match action {
            NetworkRecoveryAction::Noop => {
                let current = self.machine.current_state().clone();
                return Ok((current.clone(), current));
            }
            NetworkRecoveryAction::Offline => Input::BeginOffline,
            NetworkRecoveryAction::Probe => Input::BeginProbe,
            NetworkRecoveryAction::Restore => Input::BeginRestore,
            NetworkRecoveryAction::CleanupOnly => Input::BeginCleanup,
            NetworkRecoveryAction::ForceReconnect => Input::BeginReconnect,
        };

        let previous = self.machine.current_state().clone();
        let current = self
            .machine
            .transition(input)
            .map_err(|error| format!("invalid recovery action start: {error}"))?;
        Ok((previous, current))
    }

    pub(crate) fn complete(&mut self, succeeded: bool) -> Result<(State, State), String> {
        let previous = self.machine.current_state().clone();
        let input = if succeeded {
            Input::Succeeded
        } else {
            Input::Failed
        };
        let current = self
            .machine
            .transition(input)
            .map_err(|error| format!("invalid recovery action completion: {error}"))?;
        Ok((previous, current))
    }
}

#[cfg(test)]
mod tests {
    use yasm::StateMachineDoc;

    use super::*;

    #[test]
    fn recovery_actions_follow_explicit_execution_phases() {
        let mut tracker = RecoveryExecutionTracker::default();

        assert_eq!(
            tracker.begin(NetworkRecoveryAction::Offline).unwrap(),
            (State::Idle, State::Disconnecting)
        );
        assert_eq!(
            tracker.complete(true).unwrap(),
            (State::Disconnecting, State::Idle)
        );

        assert_eq!(
            tracker.begin(NetworkRecoveryAction::Restore).unwrap(),
            (State::Idle, State::Restoring)
        );
        assert_eq!(
            tracker.complete(true).unwrap(),
            (State::Restoring, State::Idle)
        );

        assert_eq!(
            tracker.begin(NetworkRecoveryAction::CleanupOnly).unwrap(),
            (State::Idle, State::Cleaning)
        );
        assert_eq!(
            tracker.complete(true).unwrap(),
            (State::Cleaning, State::Idle)
        );
    }

    #[test]
    fn failed_action_returns_execution_layer_to_idle() {
        let mut tracker = RecoveryExecutionTracker::default();

        tracker.begin(NetworkRecoveryAction::Probe).unwrap();
        assert_eq!(
            tracker.complete(false).unwrap(),
            (State::Probing, State::Idle)
        );
        assert_eq!(
            tracker
                .begin(NetworkRecoveryAction::ForceReconnect)
                .unwrap(),
            (State::Idle, State::Reconnecting)
        );
        assert_eq!(
            tracker.complete(true).unwrap(),
            (State::Reconnecting, State::Idle)
        );
    }

    #[test]
    fn overlapping_actions_are_rejected() {
        let mut tracker = RecoveryExecutionTracker::default();

        tracker.begin(NetworkRecoveryAction::Restore).unwrap();
        let error = tracker
            .begin(NetworkRecoveryAction::ForceReconnect)
            .expect_err("an executing restore must reject another action");

        assert!(error.contains("invalid recovery action start"));
        assert_eq!(*tracker.machine.current_state(), State::Restoring);
    }

    #[test]
    fn state_machine_documentation_contains_recovery_phases() {
        let mermaid = StateMachineDoc::<RecoveryExecutionMachine>::generate_mermaid();

        assert!(mermaid.contains("Probing"));
        assert!(mermaid.contains("Restoring"));
        assert!(mermaid.contains("Reconnecting"));
        assert!(mermaid.contains("Cleaning"));
        assert!(!mermaid.lines().any(|line| {
            line.trim_start().starts_with("Offline -->") || line.contains("--> Offline :")
        }));
        assert!(!mermaid.lines().any(|line| {
            line.trim_start().starts_with("Failed -->") || line.contains("--> Failed :")
        }));
    }
}
