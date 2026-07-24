//! The three YASM state machines the RFC-0400 executable reference was missing.
//!
//! `RecoveryMode`, `CleanupWork`, and `RetryGate` complete the normative
//! machine set alongside the pre-RFC machines in
//! [`super::super::recovery_supervisor`] and
//! [`super::super::recovery_execution`]. Each machine is isolated in its own
//! module so the generated `State` / `Input` enums do not collide, mirroring
//! the isolation pattern used by the connection supervisor.
//!
//! These machines carry only discrete transitions and their invalid-transition
//! checks. Extended context — attempt counts, retry ids, deadlines, and release
//! masks — lives with the pure reducer in [`super::translate`], never in the
//! state graph.

/// `RecoveryMode` answers whether automatic recovery may start at all.
///
/// `Terminating` is terminal for one supervisor lifetime; only a successfully
/// committed new session emits `SessionActivated`, so a later network fact
/// cannot reactivate recovery after logout.
pub(crate) mod recovery_mode {
    use yasm::define_state_machine;

    define_state_machine! {
        name: RecoveryModeMachine,
        states: {
            Active,
            LoggedOut,
            Terminating
        },
        inputs: {
            SessionActivated,
            UserLoggedOut,
            AppTerminating
        },
        initial: Active,
        transitions: {
            Active + SessionActivated => Active,
            Active + UserLoggedOut => LoggedOut,
            Active + AppTerminating => Terminating,

            LoggedOut + SessionActivated => Active,
            LoggedOut + UserLoggedOut => LoggedOut,
            LoggedOut + AppTerminating => Terminating,

            Terminating + SessionActivated => Terminating,
            Terminating + UserLoggedOut => Terminating,
            Terminating + AppTerminating => Terminating
        }
    }
}

/// `CleanupWork` holds the intent-bearing cleanup obligation.
///
/// It is separate from `OfflineWork` because teardown and future recovery
/// intent may coexist. Cleanup completion returns it to `Idle`.
pub(crate) mod cleanup_work {
    use yasm::define_state_machine;

    define_state_machine! {
        name: CleanupWorkMachine,
        states: {
            Idle,
            CleanupPending
        },
        inputs: {
            RequestCleanup,
            CompleteCleanup
        },
        initial: Idle,
        transitions: {
            Idle + RequestCleanup => CleanupPending,

            CleanupPending + RequestCleanup => CleanupPending,
            CleanupPending + CompleteCleanup => Idle
        }
    }
}

/// `RetryGate` separates "work is still required" from "work may start now".
///
/// `Ready` is eligible, `BackingOff` owns one exact retry deadline, and
/// `Parked` waits for a trigger matching its recorded release mask after a
/// precondition-family verdict or an explicit pause. Failure inputs reach only
/// `Ready` gates by construction; the missing rows are intentionally undefined
/// so an out-of-band failure is an invalid transition rather than a silent
/// self-loop.
pub(crate) mod retry_gate {
    use yasm::define_state_machine;

    define_state_machine! {
        name: RetryGateMachine,
        states: {
            Ready,
            BackingOff,
            Parked
        },
        inputs: {
            RetryableFailure,
            TerminalFailure,
            ExplicitPause,
            RetryDeadlineExpired,
            NewMaterialTrigger
        },
        initial: Ready,
        transitions: {
            Ready + RetryableFailure => BackingOff,
            Ready + TerminalFailure => Parked,
            Ready + ExplicitPause => Parked,
            Ready + NewMaterialTrigger => Ready,

            BackingOff + RetryDeadlineExpired => Ready,
            BackingOff + NewMaterialTrigger => Ready,
            BackingOff + ExplicitPause => Parked,

            Parked + NewMaterialTrigger => Ready,
            Parked + ExplicitPause => Parked
        }
    }
}

#[cfg(test)]
mod tests {
    use yasm::StateMachine;

    use super::{cleanup_work, recovery_mode, retry_gate};

    /// Assert that a machine's full transition table equals the expected set of
    /// `(current, input, next)` tuples and that no undefined pair transitions.
    fn assert_transition_table<SM: StateMachine>(expected: &[(SM::State, SM::Input, SM::State)]) {
        let mut defined = 0usize;
        for state in SM::states() {
            for input in SM::inputs() {
                match SM::next_state(&state, &input) {
                    Some(next) => {
                        defined += 1;
                        assert!(
                            expected
                                .iter()
                                .any(|(s, i, n)| *s == state && *i == input && *n == next),
                            "unexpected transition {:?} + {:?} => {:?}",
                            state,
                            input,
                            next,
                        );
                    }
                    None => assert!(
                        !expected.iter().any(|(s, i, _)| *s == state && *i == input),
                        "expected transition missing for {:?} + {:?}",
                        state,
                        input,
                    ),
                }
            }
        }
        assert_eq!(defined, expected.len(), "transition count mismatch");
    }

    #[test]
    fn recovery_mode_transition_table_is_exact() {
        use recovery_mode::{Input as I, RecoveryModeMachine as M, State as S};
        assert_eq!(<M as StateMachine>::initial_state(), S::Active);
        assert_transition_table::<M>(&[
            (S::Active, I::SessionActivated, S::Active),
            (S::Active, I::UserLoggedOut, S::LoggedOut),
            (S::Active, I::AppTerminating, S::Terminating),
            (S::LoggedOut, I::SessionActivated, S::Active),
            (S::LoggedOut, I::UserLoggedOut, S::LoggedOut),
            (S::LoggedOut, I::AppTerminating, S::Terminating),
            (S::Terminating, I::SessionActivated, S::Terminating),
            (S::Terminating, I::UserLoggedOut, S::Terminating),
            (S::Terminating, I::AppTerminating, S::Terminating),
        ]);
    }

    #[test]
    fn cleanup_work_transition_table_is_exact() {
        use cleanup_work::{CleanupWorkMachine as M, Input as I, State as S};
        assert_eq!(<M as StateMachine>::initial_state(), S::Idle);
        assert_transition_table::<M>(&[
            (S::Idle, I::RequestCleanup, S::CleanupPending),
            (S::CleanupPending, I::RequestCleanup, S::CleanupPending),
            (S::CleanupPending, I::CompleteCleanup, S::Idle),
        ]);
    }

    #[test]
    fn retry_gate_transition_table_is_exact() {
        use retry_gate::{Input as I, RetryGateMachine as M, State as S};
        assert_eq!(<M as StateMachine>::initial_state(), S::Ready);
        assert_transition_table::<M>(&[
            (S::Ready, I::RetryableFailure, S::BackingOff),
            (S::Ready, I::TerminalFailure, S::Parked),
            (S::Ready, I::ExplicitPause, S::Parked),
            (S::Ready, I::NewMaterialTrigger, S::Ready),
            (S::BackingOff, I::RetryDeadlineExpired, S::Ready),
            (S::BackingOff, I::NewMaterialTrigger, S::Ready),
            (S::BackingOff, I::ExplicitPause, S::Parked),
            (S::Parked, I::NewMaterialTrigger, S::Ready),
            (S::Parked, I::ExplicitPause, S::Parked),
        ]);
    }

    #[test]
    fn retry_gate_failures_never_reach_backing_off_or_parked() {
        // Failure inputs are dispatched only to Ready gates; the machine leaves
        // these pairs undefined so a defect is an invalid transition.
        use retry_gate::{Input as I, RetryGateMachine as M, State as S};
        for state in [S::BackingOff, S::Parked] {
            assert!(<M as StateMachine>::next_state(&state, &I::RetryableFailure).is_none());
            assert!(<M as StateMachine>::next_state(&state, &I::TerminalFailure).is_none());
        }
    }
}
