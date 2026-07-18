//! Synchronous engine tests for the responsive recovery supervisor.
//!
//! These exercise the RFC-0400 policy engine directly — no runtime, no wall
//! clock — by feeding [`tp::Input`]s with an explicit monotonic `now` and an
//! injected deterministic entropy source. They cover the Phase 2 acceptance
//! invariants that are decidable in the pure engine: snapshot acceptance,
//! offline hysteresis, mode gating, the completion procedure, retry gating,
//! escalation, cleanup supersession, and the lifecycle profile.

use std::time::Duration;

use super::ConnectionSupervisor;
use crate::lifecycle::recovery_policy::classification::FixedEntropy;
use crate::lifecycle::recovery_policy::diagnosis::{EffectDiagnosis, EffectKind, EffectOutcome};
use crate::lifecycle::recovery_policy::translate as tp;

fn ungated() -> ConnectionSupervisor {
    ConnectionSupervisor::new_with_entropy(
        tp::LifecycleProfile::Ungated,
        Box::new(FixedEntropy::constant(0.5)),
    )
}

fn gated() -> ConnectionSupervisor {
    ConnectionSupervisor::new_with_entropy(
        tp::LifecycleProfile::Gated,
        Box::new(FixedEntropy::constant(0.5)),
    )
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn snapshot(epoch: u64, sequence: u64, path: tp::SemanticPath) -> tp::Input {
    tp::Input::NetworkSnapshot {
        source_epoch: epoch,
        sequence,
        semantic_path: path,
        route_fingerprint: 0,
    }
}

fn online(epoch: u64, sequence: u64) -> tp::Input {
    snapshot(epoch, sequence, tp::SemanticPath::Online)
}

fn offline(epoch: u64, sequence: u64) -> tp::Input {
    snapshot(epoch, sequence, tp::SemanticPath::Offline)
}

fn has_arm(outcome: &super::AcceptOutcome, key: tp::TimerId) -> bool {
    outcome
        .timer_ops
        .iter()
        .any(|op| matches!(op, super::TimerOp::Arm { key: k, .. } if *k == key))
}

// -- invariant 1: snapshot acceptance ordering ------------------------------

#[test]
fn duplicate_and_stale_snapshots_cannot_move_path_state() {
    let mut s = ungated();
    assert!(s.accept(online(1, 2), ms(0)).advanced);
    assert_eq!(s.view().network_path, tp::NetworkPathState::Online);

    // A lower sequence in the same epoch is stale and discarded.
    let stale = s.accept(offline(1, 1), ms(1));
    assert!(!stale.advanced);
    assert_eq!(s.view().network_path, tp::NetworkPathState::Online);

    // A newer sequence with identical semantics is a structural duplicate: it
    // records the sequence but advances no revision and rearms no candidate.
    let dup = s.accept(online(1, 3), ms(2));
    assert!(!dup.advanced);
    assert_eq!(s.view().network_path, tp::NetworkPathState::Online);
    assert_eq!(s.view().last_snapshot.unwrap().sequence, 3);

    // A newer epoch begins a fresh monotonic sequence and is accepted.
    let reincarnate = s.accept(offline(2, 1), ms(3));
    assert!(reincarnate.advanced);
    assert_eq!(
        s.view().network_path,
        tp::NetworkPathState::OfflineCandidate
    );
}

// -- offline hysteresis (invariants 2, 3) -----------------------------------

#[test]
fn offline_candidate_arms_grace_and_permits_existing_traffic() {
    let mut s = ungated();
    s.accept(online(1, 1), ms(0));
    let out = s.accept(offline(1, 2), ms(10));

    assert_eq!(
        s.view().network_path,
        tp::NetworkPathState::OfflineCandidate
    );
    assert!(has_arm(&out, tp::TimerId::OfflineCandidate));
    // A candidate allows existing-session traffic but no new negotiation and no
    // teardown barrier.
    assert_eq!(s.send_policy(), tp::SendProjection::ExistingOnly);
    assert_eq!(s.composite_action(), None);
}

#[test]
fn offline_grace_expiry_commits_and_derives_confirmed_disconnect() {
    let mut s = ungated();
    s.accept(online(1, 1), ms(0));
    s.accept(offline(1, 2), ms(10));
    let candidate = s.view().offline_candidate.unwrap().candidate_id;

    let out = s.accept(
        tp::Input::OfflineGraceExpired {
            candidate_id: candidate,
        },
        ms(410),
    );
    assert!(out.advanced);
    assert_eq!(s.view().network_path, tp::NetworkPathState::Offline);
    assert_eq!(
        s.view().offline_work,
        tp::OfflineWorkState::DisconnectPending
    );
    assert_eq!(
        s.composite_action(),
        Some(tp::Action::ConfirmedOfflineDisconnect)
    );
    assert_eq!(s.send_policy(), tp::SendProjection::Blocked);
    // The offline-disconnect obligation arms one overall teardown deadline.
    assert!(has_arm(
        &out,
        tp::TimerId::TeardownOverall(tp::TeardownDomain::OfflineDisconnect)
    ));
}

#[test]
fn online_before_grace_rolls_back_without_disconnect() {
    let mut s = ungated();
    s.accept(online(1, 1), ms(0));
    s.accept(offline(1, 2), ms(10));
    let candidate = s.view().offline_candidate.unwrap().candidate_id;

    let out = s.accept(online(1, 3), ms(100));
    assert_eq!(s.view().network_path, tp::NetworkPathState::Online);
    assert!(s.view().offline_candidate.is_none());
    assert_eq!(s.view().offline_work, tp::OfflineWorkState::Idle);
    // The candidate timer is cancelled by the rollback.
    assert!(out.timer_ops.iter().any(|op| matches!(
        op,
        super::TimerOp::Cancel {
            key: tp::TimerId::OfflineCandidate
        }
    )));

    // A stale expiry for the abandoned candidate is inert.
    let stale = s.accept(
        tp::Input::OfflineGraceExpired {
            candidate_id: candidate,
        },
        ms(410),
    );
    assert!(!stale.advanced);
    assert_eq!(s.view().network_path, tp::NetworkPathState::Online);
}

// -- mode gating (invariant 5) ----------------------------------------------

#[test]
fn logout_gates_recovery_and_is_not_reactivated_by_network_facts() {
    let mut s = ungated();
    s.accept(
        tp::Input::CleanupRequested {
            reason: tp::CleanupReason::UserLogout,
        },
        ms(0),
    );
    assert_eq!(s.view().recovery_mode, tp::RecoveryModeState::LoggedOut);
    assert_eq!(s.view().cleanup_work, tp::CleanupWorkState::CleanupPending);

    // A later online fact cannot reactivate recovery.
    s.accept(online(1, 5), ms(10));
    assert_eq!(s.view().recovery_mode, tp::RecoveryModeState::LoggedOut);
    assert_eq!(s.view().recovery_intent, tp::RecoveryIntentState::Idle);
    // Cleanup remains the only admissible action.
    assert_eq!(s.composite_action(), Some(tp::Action::Cleanup));
    assert_eq!(s.send_policy(), tp::SendProjection::Blocked);
}

// -- duplicate foreground (invariant 6) -------------------------------------

#[test]
fn cold_and_duplicate_foreground_create_no_recovery_work() {
    let mut s = ungated();
    // First authoritative phase from Unknown does not fabricate recovery work.
    s.accept(tp::Input::AppEnteredForeground, ms(0));
    assert_eq!(s.view().app_phase, tp::AppPhaseState::Foreground);
    assert_eq!(s.view().recovery_intent, tp::RecoveryIntentState::Idle);

    // A duplicate foreground is a self-loop.
    let dup = s.accept(tp::Input::AppEnteredForeground, ms(1));
    assert!(!dup.advanced);
    assert_eq!(s.view().recovery_intent, tp::RecoveryIntentState::Idle);
}

// -- cleanup supersession (invariants 4, 31) --------------------------------

#[test]
fn cleanup_supersedes_pending_recovery_by_revision() {
    let mut s = ungated();
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Reconnect,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(0),
    );
    assert_eq!(
        s.view().recovery_intent,
        tp::RecoveryIntentState::ReconnectPending
    );

    s.accept(
        tp::Input::CleanupRequested {
            reason: tp::CleanupReason::ManualReset,
        },
        ms(1),
    );
    // Recovery intent is superseded; cleanup owns the slot.
    assert_eq!(s.view().recovery_intent, tp::RecoveryIntentState::Idle);
    assert_eq!(s.view().cleanup_work, tp::CleanupWorkState::CleanupPending);
    assert_eq!(s.composite_action(), Some(tp::Action::Cleanup));
}

// -- completion procedure: success acknowledgement --------------------------

#[test]
fn probe_success_acknowledges_recovery_intent() {
    let mut s = ungated();
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Probe,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = s.maybe_start_effect().expect("probe should start");
    assert_eq!(started.kind, EffectKind::Probe);
    assert_eq!(s.view().execution, tp::ExecutionState::Probing);

    s.accept(
        tp::Input::EffectCompleted {
            action_id: started.action_id,
            kind: EffectKind::Probe,
            policy_revision: started.captured_revision,
            outcome: EffectOutcome::Succeeded,
        },
        ms(2),
    );
    assert_eq!(s.view().execution, tp::ExecutionState::Idle);
    assert_eq!(s.view().recovery_intent, tp::RecoveryIntentState::Idle);
    assert!(s.view().recovery_record.is_none());
}

// -- completion procedure: stale completion (invariant 9) -------------------

#[test]
fn stale_completion_cannot_acknowledge_work() {
    let mut s = ungated();
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Probe,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = s.maybe_start_effect().expect("probe should start");

    // Wrong action id: discarded, execution unchanged.
    let out = s.accept(
        tp::Input::EffectCompleted {
            action_id: started.action_id + 99,
            kind: EffectKind::Probe,
            policy_revision: started.captured_revision,
            outcome: EffectOutcome::Succeeded,
        },
        ms(2),
    );
    assert!(!out.advanced);
    assert_eq!(s.view().execution, tp::ExecutionState::Probing);
    assert_eq!(
        s.view().recovery_intent,
        tp::RecoveryIntentState::ProbePending
    );
}

// -- retry gating (invariant 10) --------------------------------------------

#[test]
fn availability_failure_backs_off_and_only_matching_deadline_rearms() {
    let mut s = ungated();
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Probe,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = s.maybe_start_effect().expect("probe should start");

    let out = s.accept(
        tp::Input::EffectCompleted {
            action_id: started.action_id,
            kind: EffectKind::Probe,
            policy_revision: started.captured_revision,
            outcome: EffectOutcome::Failed {
                diagnosis: EffectDiagnosis::PathUnreachable {
                    stage: "connect".into(),
                },
            },
        },
        ms(2),
    );
    let record = s.view().recovery_record.clone().unwrap();
    assert_eq!(record.gate, tp::RetryGateState::BackingOff);
    assert_eq!(record.attempt, 1);
    assert!(has_arm(
        &out,
        tp::TimerId::FailureBackoff(tp::RetryDomain::Recovery)
    ));
    // Not Ready, so no action is selectable: it cannot hot-loop.
    assert_eq!(s.composite_action(), None);

    // A stale retry deadline (wrong id) is inert.
    let stale = s.accept(
        tp::Input::RetryDeadlineExpired {
            domain: tp::RetryDomain::Recovery,
            work_revision: record.work_revision,
            retry_id: record.retry_id + 7,
        },
        ms(3),
    );
    assert!(!stale.advanced);
    assert_eq!(
        s.view().recovery_record.as_ref().unwrap().gate,
        tp::RetryGateState::BackingOff
    );

    // The matching deadline makes it Ready again.
    s.accept(
        tp::Input::RetryDeadlineExpired {
            domain: tp::RetryDomain::Recovery,
            work_revision: record.work_revision,
            retry_id: record.retry_id,
        },
        ms(4),
    );
    assert_eq!(
        s.view().recovery_record.as_ref().unwrap().gate,
        tp::RetryGateState::Ready
    );
    assert_eq!(s.composite_action(), Some(tp::Action::Probe));
}

// -- escalation retains the causal work revision ----------------------------

#[test]
fn transport_impairment_escalates_probe_to_restore_retaining_revision() {
    let mut s = ungated();
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Probe,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let work_revision = s.view().recovery_record.as_ref().unwrap().work_revision;
    let started = s.maybe_start_effect().expect("probe should start");

    s.accept(
        tp::Input::EffectCompleted {
            action_id: started.action_id,
            kind: EffectKind::Probe,
            policy_revision: started.captured_revision,
            outcome: EffectOutcome::Failed {
                diagnosis: EffectDiagnosis::TransportImpaired {
                    scopes: vec!["data".into()],
                },
            },
        },
        ms(2),
    );
    assert_eq!(s.view().execution, tp::ExecutionState::Idle);
    assert_eq!(
        s.view().recovery_intent,
        tp::RecoveryIntentState::RestorePending
    );
    let record = s.view().recovery_record.as_ref().unwrap();
    // A fresh Ready gate at the *retained* causal work revision.
    assert_eq!(record.gate, tp::RetryGateState::Ready);
    assert_eq!(record.attempt, 0);
    assert_eq!(record.work_revision, work_revision);
}

// -- precondition failure parks with a mask ---------------------------------

#[test]
fn auth_rejection_parks_recovery_until_a_clearing_trigger() {
    let mut s = ungated();
    s.accept(online(1, 1), ms(0));
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Restore,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = s.maybe_start_effect().expect("restore should start");

    s.accept(
        tp::Input::EffectCompleted {
            action_id: started.action_id,
            kind: EffectKind::Restore,
            policy_revision: started.captured_revision,
            outcome: EffectOutcome::Failed {
                diagnosis: EffectDiagnosis::AuthRejected {
                    kind: "expired".into(),
                },
            },
        },
        ms(2),
    );
    let record = s.view().recovery_record.as_ref().unwrap();
    assert_eq!(record.gate, tp::RetryGateState::Parked);
    assert!(!record.release_mask.is_empty());
    assert_eq!(s.composite_action(), None);

    // A committed session generation clears the auth park and re-arms recovery.
    s.accept(
        tp::Input::SessionActivated {
            session_generation: 1,
        },
        ms(3),
    );
    assert_eq!(
        s.view().recovery_record.as_ref().unwrap().gate,
        tp::RetryGateState::Ready
    );
}

// -- lifecycle profile / bootstrap (invariant 7) ----------------------------

#[test]
fn gated_profile_denies_eligibility_until_foreground() {
    let mut g = gated();
    assert!(g.bootstrap_arm(ms(0)).is_some());

    g.accept(online(1, 1), ms(0));
    g.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Restore,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    // Unknown phase under Gated grants no eligibility.
    assert_eq!(g.composite_action(), None);

    // The first authoritative foreground makes recovery eligible and cancels the
    // bootstrap deadline.
    let out = g.accept(tp::Input::AppEnteredForeground, ms(2));
    assert!(out.timer_ops.iter().any(|op| matches!(
        op,
        super::TimerOp::Cancel {
            key: tp::TimerId::BootstrapPhase
        }
    )));
    assert_eq!(g.composite_action(), Some(tp::Action::Restore));
}

#[test]
fn gated_bootstrap_expiry_keeps_profile_gated() {
    let mut g = gated();
    let out = g.accept(tp::Input::BootstrapPhaseDeadlineExpired, ms(5_000));
    assert!(
        out.status
            .iter()
            .any(|r| matches!(r, tp::StatusRecord::BootstrapDeadlineElapsed))
    );
    // Still Unknown, still ineligible: eligibility is never granted silently.
    assert_eq!(g.view().app_phase, tp::AppPhaseState::Unknown);
    assert_eq!(g.composite_action(), None);
}

// -- terminating cleanup arms shutdown; send policy blocked -----------------

#[test]
fn app_terminating_enters_terminating_and_blocks_sends() {
    let mut s = ungated();
    let out = s.accept(tp::Input::ShutdownRequested, ms(0));
    assert_eq!(s.view().recovery_mode, tp::RecoveryModeState::Terminating);
    assert_eq!(s.view().cleanup_work, tp::CleanupWorkState::CleanupPending);
    assert!(has_arm(&out, tp::TimerId::ShutdownOverall));
    assert!(has_arm(
        &out,
        tp::TimerId::TeardownOverall(tp::TeardownDomain::Cleanup)
    ));
    assert_eq!(s.send_policy(), tp::SendProjection::Blocked);
}

// -- preemption records a cancel reason (invariant 27 slot release) ---------

#[test]
fn cleanup_preempts_a_running_probe() {
    let mut s = ungated();
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Probe,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = s.maybe_start_effect().expect("probe should start");
    assert_eq!(s.view().execution, tp::ExecutionState::Probing);

    let out = s.accept(
        tp::Input::CleanupRequested {
            reason: tp::CleanupReason::ManualReset,
        },
        ms(2),
    );
    // Cancellation is requested but the execution slot is not directly mutated.
    assert!(out.cancel_effect);
    assert_eq!(s.view().execution, tp::ExecutionState::Probing);
    assert_eq!(
        s.view().effect.as_ref().unwrap().cancel_reason,
        Some(tp::CancelReason::PreemptedByCleanup)
    );

    // The recorded cancellation completes the slot; cleanup then runs.
    s.accept(
        tp::Input::EffectCompleted {
            action_id: started.action_id,
            kind: EffectKind::Probe,
            policy_revision: started.captured_revision,
            outcome: EffectOutcome::Cancelled,
        },
        ms(3),
    );
    assert_eq!(s.view().execution, tp::ExecutionState::Idle);
    assert_eq!(s.composite_action(), Some(tp::Action::Cleanup));
}
