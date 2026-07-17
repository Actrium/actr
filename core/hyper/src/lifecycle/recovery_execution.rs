//! Deterministic execution lifecycle for selected network recovery actions.
//!
//! [`ConnectionSupervisor`](super::ConnectionSupervisor) is the policy FSM: it
//! consumes normalized facts and selects one action. This second FSM starts
//! after that selection. It rejects overlapping recovery actions and makes
//! every executing phase explicit without putting asynchronous side effects
//! inside either state machine.

use yasm::{StateMachineInstance, define_state_machine};

use super::NetworkRecoveryAction;

define_state_machine! {
    name: RecoveryExecutionMachine,
    states: {
        Ready,
        GoingOffline,
        Offline,
        Probing,
        Restoring,
        Reconnecting,
        Cleaning,
        Quiescent,
        Failed
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
    initial: Ready,
    transitions: {
        Ready + BeginOffline => GoingOffline,
        Ready + BeginProbe => Probing,
        Ready + BeginRestore => Restoring,
        Ready + BeginReconnect => Reconnecting,
        Ready + BeginCleanup => Cleaning,

        Offline + BeginOffline => GoingOffline,
        Offline + BeginProbe => Probing,
        Offline + BeginRestore => Restoring,
        Offline + BeginReconnect => Reconnecting,
        Offline + BeginCleanup => Cleaning,

        Quiescent + BeginOffline => GoingOffline,
        Quiescent + BeginProbe => Probing,
        Quiescent + BeginRestore => Restoring,
        Quiescent + BeginReconnect => Reconnecting,
        Quiescent + BeginCleanup => Cleaning,

        Failed + BeginOffline => GoingOffline,
        Failed + BeginProbe => Probing,
        Failed + BeginRestore => Restoring,
        Failed + BeginReconnect => Reconnecting,
        Failed + BeginCleanup => Cleaning,

        GoingOffline + Succeeded => Offline,
        Probing + Succeeded => Ready,
        Restoring + Succeeded => Ready,
        Reconnecting + Succeeded => Ready,
        Cleaning + Succeeded => Quiescent,

        GoingOffline + Failed => Failed,
        Probing + Failed => Failed,
        Restoring + Failed => Failed,
        Reconnecting + Failed => Failed,
        Cleaning + Failed => Failed
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
            (State::Ready, State::GoingOffline)
        );
        assert_eq!(
            tracker.complete(true).unwrap(),
            (State::GoingOffline, State::Offline)
        );

        assert_eq!(
            tracker.begin(NetworkRecoveryAction::Restore).unwrap(),
            (State::Offline, State::Restoring)
        );
        assert_eq!(
            tracker.complete(true).unwrap(),
            (State::Restoring, State::Ready)
        );

        assert_eq!(
            tracker.begin(NetworkRecoveryAction::CleanupOnly).unwrap(),
            (State::Ready, State::Cleaning)
        );
        assert_eq!(
            tracker.complete(true).unwrap(),
            (State::Cleaning, State::Quiescent)
        );
    }

    #[test]
    fn failed_action_can_be_recovered_by_a_later_action() {
        let mut tracker = RecoveryExecutionTracker::default();

        tracker.begin(NetworkRecoveryAction::Probe).unwrap();
        assert_eq!(
            tracker.complete(false).unwrap(),
            (State::Probing, State::Failed)
        );
        assert_eq!(
            tracker
                .begin(NetworkRecoveryAction::ForceReconnect)
                .unwrap(),
            (State::Failed, State::Reconnecting)
        );
        assert_eq!(
            tracker.complete(true).unwrap(),
            (State::Reconnecting, State::Ready)
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
    }
}
