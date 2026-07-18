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
fn inv1_duplicate_and_stale_snapshots_cannot_move_path_state() {
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
fn inv3_offline_candidate_arms_grace_and_permits_existing_traffic() {
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
fn inv29_offline_grace_expiry_commits_and_derives_confirmed_disconnect() {
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
    // `DisconnectPending` implies `NetworkPath == Offline` at reconciliation
    // entry (invariant 29): both land in the same accepted step.
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
fn inv2_online_cancels_a_running_confirmed_disconnect_and_derives_restoration() {
    let mut s = ungated();
    s.accept(online(1, 1), ms(0));
    s.accept(offline(1, 2), ms(10));
    let candidate = s.view().offline_candidate.unwrap().candidate_id;
    s.accept(
        tp::Input::OfflineGraceExpired {
            candidate_id: candidate,
        },
        ms(410),
    );
    let started = s
        .maybe_start_effect(ms(411))
        .expect("confirmed disconnect should start");
    assert_eq!(s.view().execution, tp::ExecutionState::Disconnecting);

    // A rollback while the disconnect effect is already running must cancel
    // it at its next commit boundary rather than let it complete, and derive
    // immediate restoration since no recovery is otherwise pending.
    let out = s.accept(online(1, 3), ms(420));
    assert!(out.cancel_effect);
    assert_eq!(
        s.view().effect.as_ref().unwrap().cancel_reason,
        Some(tp::CancelReason::PathRecovered)
    );
    assert_eq!(s.view().network_path, tp::NetworkPathState::Online);
    assert_eq!(
        s.view().recovery_intent,
        tp::RecoveryIntentState::RestorePending
    );

    // The disconnect effect's own (now-stale) completion must not undo the
    // rollback or re-arm anything: its captured revision is not newer than
    // the offline record, so the offline-work extinguishment still applies,
    // but the network path and recovery intent are unaffected.
    s.accept(
        tp::Input::EffectCompleted {
            action_id: started.action_id,
            kind: EffectKind::ConfirmedOfflineDisconnect,
            policy_revision: started.captured_revision,
            outcome: EffectOutcome::Cancelled,
        },
        ms(421),
    );
    assert_eq!(s.view().network_path, tp::NetworkPathState::Online);
    assert_eq!(
        s.view().recovery_intent,
        tp::RecoveryIntentState::RestorePending
    );
}

#[test]
fn inv2_online_before_grace_rolls_back_without_disconnect() {
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
fn inv5_logout_gates_recovery_and_is_not_reactivated_by_network_facts() {
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
fn inv6_cold_and_duplicate_foreground_create_no_recovery_work() {
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
fn inv4_cleanup_supersedes_pending_recovery_by_revision() {
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

#[test]
fn inv4_cleanup_completion_cannot_acknowledge_a_later_recovery_fact() {
    let mut s = ungated();
    s.accept(
        tp::Input::CleanupRequested {
            reason: tp::CleanupReason::ManualReset,
        },
        ms(0),
    );
    let started = s.maybe_start_effect(ms(1)).expect("cleanup should start");
    assert_eq!(s.view().execution, tp::ExecutionState::Cleaning);

    // A recovery fact arrives *after* cleanup has already started running.
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Restore,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(2),
    );
    assert_eq!(
        s.view().recovery_intent,
        tp::RecoveryIntentState::RestorePending
    );

    // Cleanup's own completion must not touch that later recovery fact.
    s.accept(
        tp::Input::EffectCompleted {
            action_id: started.action_id,
            kind: EffectKind::Cleanup,
            policy_revision: started.captured_revision,
            outcome: EffectOutcome::Succeeded,
        },
        ms(3),
    );
    assert_eq!(s.view().cleanup_work, tp::CleanupWorkState::Idle);
    assert_eq!(
        s.view().recovery_intent,
        tp::RecoveryIntentState::RestorePending,
        "cleanup completion must not acknowledge a later recovery fact"
    );
    assert_eq!(s.composite_action(), Some(tp::Action::Restore));
}

// -- session activated during cleanup (invariant 31) ------------------------

#[test]
fn inv31_session_activated_during_cleanup_derives_a_post_cleanup_obligation() {
    let mut s = ungated();
    s.accept(
        tp::Input::CleanupRequested {
            reason: tp::CleanupReason::ManualReset,
        },
        ms(0),
    );
    let started = s.maybe_start_effect(ms(1)).expect("cleanup should start");
    let cleanup_revision = started.captured_revision;

    // A new session commits authoritatively while the older cleanup is still
    // running.
    s.accept(
        tp::Input::SessionActivated {
            session_generation: 1,
        },
        ms(2),
    );
    assert_eq!(
        s.view().recovery_intent,
        tp::RecoveryIntentState::RestorePending
    );
    let recovery_record = s
        .view()
        .recovery_record
        .clone()
        .expect("a new recovery obligation should be derived");
    assert!(
        recovery_record.work_revision > cleanup_revision,
        "the new session's obligation must be recorded at a revision after \
         the running cleanup's own captured revision"
    );

    // The older cleanup's own completion must not extinguish that
    // post-cleanup obligation.
    s.accept(
        tp::Input::EffectCompleted {
            action_id: started.action_id,
            kind: EffectKind::Cleanup,
            policy_revision: cleanup_revision,
            outcome: EffectOutcome::Succeeded,
        },
        ms(3),
    );
    assert_eq!(s.view().cleanup_work, tp::CleanupWorkState::Idle);
    assert_eq!(
        s.view().recovery_intent,
        tp::RecoveryIntentState::RestorePending
    );
    assert_eq!(s.composite_action(), Some(tp::Action::Restore));
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
    let started = s.maybe_start_effect(ms(2)).expect("probe should start");
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
fn inv9_stale_completion_cannot_acknowledge_work() {
    let mut s = ungated();
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Probe,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = s.maybe_start_effect(ms(2)).expect("probe should start");

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

#[test]
fn inv8_execution_is_single_flight_while_supervisor_stays_responsive() {
    let mut s = ungated();
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Probe,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = s.maybe_start_effect(ms(2)).expect("probe should start");
    assert_eq!(s.view().execution, tp::ExecutionState::Probing);

    // The supervisor keeps accepting and advancing its view while the probe
    // runs (it stays responsive)...
    let out = s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Restore,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(3),
    );
    assert!(out.advanced);
    assert_eq!(
        s.view().recovery_intent,
        tp::RecoveryIntentState::RestorePending
    );
    // ...but a second effect never starts while one is already executing: the
    // slot is single-flight lifecycle-wide.
    assert!(s.maybe_start_effect(ms(3)).is_none());
    assert_eq!(s.view().execution, tp::ExecutionState::Probing);
    assert_eq!(
        s.view().effect.as_ref().unwrap().action_id,
        started.action_id
    );
}

#[test]
fn inv9_weaker_effect_completion_cannot_acknowledge_stronger_intent() {
    let mut s = ungated();
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Probe,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = s.maybe_start_effect(ms(2)).expect("probe should start");

    // The intent is escalated to a stronger ask while the (weaker) probe is
    // still executing; single-flight keeps the same probe running.
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Restore,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(3),
    );
    assert_eq!(
        s.view().recovery_intent,
        tp::RecoveryIntentState::RestorePending
    );
    assert_eq!(s.view().execution, tp::ExecutionState::Probing);

    // The original (weaker) probe now completes successfully with its own
    // captured action id, kind, and revision — it must not acknowledge the
    // newer, stronger `RestorePending` intent it was never running for.
    s.accept(
        tp::Input::EffectCompleted {
            action_id: started.action_id,
            kind: EffectKind::Probe,
            policy_revision: started.captured_revision,
            outcome: EffectOutcome::Succeeded,
        },
        ms(4),
    );
    assert_eq!(
        s.view().recovery_intent,
        tp::RecoveryIntentState::RestorePending,
        "a weaker completed effect must not acknowledge a newer, stronger intent"
    );
    assert_eq!(s.composite_action(), Some(tp::Action::Restore));
}

// -- retry gating (invariant 10) --------------------------------------------

#[test]
fn inv10_availability_failure_backs_off_and_only_matching_deadline_rearms() {
    let mut s = ungated();
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Probe,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = s.maybe_start_effect(ms(2)).expect("probe should start");

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
    let started = s.maybe_start_effect(ms(2)).expect("probe should start");

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
fn inv10_auth_rejection_parks_recovery_until_a_clearing_trigger() {
    let mut s = ungated();
    s.accept(online(1, 1), ms(0));
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Restore,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = s.maybe_start_effect(ms(2)).expect("restore should start");

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

// -- classification table alone determines the verdict (invariant 25) ------

#[test]
fn inv25_classification_table_alone_determines_the_verdict() {
    // Two structurally distinct diagnoses for the *same* effect kind and
    // attempt produce different verdicts purely from the per-kind
    // classification table in the translation layer. `EffectOutcome::Failed`
    // carries only a typed `diagnosis`, never a verdict, so the effect that
    // reports it never has a say in retry, escalation, park, or abandon.
    let mut availability = ungated();
    availability.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Probe,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = availability
        .maybe_start_effect(ms(2))
        .expect("probe should start");
    availability.accept(
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
    assert_eq!(
        availability.view().recovery_record.as_ref().unwrap().gate,
        tp::RetryGateState::BackingOff,
        "an availability-family diagnosis backs off"
    );

    let mut precondition = ungated();
    precondition.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Probe,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = precondition
        .maybe_start_effect(ms(2))
        .expect("probe should start");
    precondition.accept(
        tp::Input::EffectCompleted {
            action_id: started.action_id,
            kind: EffectKind::Probe,
            policy_revision: started.captured_revision,
            outcome: EffectOutcome::Failed {
                diagnosis: EffectDiagnosis::AuthRejected {
                    kind: "expired".into(),
                },
            },
        },
        ms(2),
    );
    assert_eq!(
        precondition.view().recovery_record.as_ref().unwrap().gate,
        tp::RetryGateState::Parked,
        "a precondition-family diagnosis parks instead"
    );
}

// -- lifecycle profile / bootstrap (invariant 7) ----------------------------

#[test]
fn inv7_gated_profile_denies_eligibility_until_foreground() {
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
fn inv7_gated_bootstrap_expiry_keeps_profile_gated() {
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

#[test]
fn inv7_gated_profile_background_gates_recovery_but_preserves_intent() {
    let mut g = gated();
    // Foreground once, establishing eligibility over a healthy online path.
    g.accept(tp::Input::AppEnteredForeground, ms(0));
    g.accept(online(1, 1), ms(1));

    // An explicit reconnect request is retained while still eligible...
    g.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Restore,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(2),
    );
    assert_eq!(
        g.view().recovery_intent,
        tp::RecoveryIntentState::RestorePending
    );

    // ...and entering background re-gates new active recovery exactly like the
    // pre-foreground `Unknown` phase does, without discarding the intent or
    // touching the healthy path.
    g.accept(tp::Input::AppEnteredBackground, ms(3));
    assert_eq!(g.composite_action(), None);
    assert_eq!(
        g.view().recovery_intent,
        tp::RecoveryIntentState::RestorePending,
        "background must preserve a pending recovery intent, not discard it"
    );
    assert_eq!(g.view().network_path, tp::NetworkPathState::Online);

    // Foreground derives its own (weaker, elapsed-time-based) probe/reconnect
    // request on top of whatever is already pending; the stronger explicit
    // `RestorePending` must not be downgraded by it.
    g.accept(tp::Input::AppEnteredForeground, ms(4));
    assert_eq!(g.composite_action(), Some(tp::Action::Restore));
}

// -- terminating cleanup arms shutdown; send policy blocked -----------------

#[test]
fn inv30_app_terminating_enters_terminating_and_blocks_sends() {
    let mut s = ungated();
    let out = s.accept(tp::Input::ShutdownRequested, ms(0));
    assert_eq!(s.view().recovery_mode, tp::RecoveryModeState::Terminating);
    assert_eq!(s.view().cleanup_work, tp::CleanupWorkState::CleanupPending);
    // Shutdown terminates the supervisor at one overall deadline.
    assert!(has_arm(&out, tp::TimerId::ShutdownOverall));
    assert!(has_arm(
        &out,
        tp::TimerId::TeardownOverall(tp::TeardownDomain::Cleanup)
    ));
    assert_eq!(s.send_policy(), tp::SendProjection::Blocked);

    // `Terminating` also cannot be reactivated by a later network fact
    // (invariant 5's other mode, alongside `LoggedOut`).
    s.accept(online(1, 1), ms(1));
    assert_eq!(s.view().recovery_mode, tp::RecoveryModeState::Terminating);
    assert_eq!(s.view().recovery_intent, tp::RecoveryIntentState::Idle);
    assert_eq!(s.send_policy(), tp::SendProjection::Blocked);
}

#[test]
fn inv28_fact_reversal_extinguishes_only_its_own_domain() {
    let mut s = ungated();
    s.accept(online(1, 1), ms(0));
    s.accept(offline(1, 2), ms(10));
    // An unrelated cleanup obligation is independently pending alongside the
    // offline candidate.
    s.accept(
        tp::Input::CleanupRequested {
            reason: tp::CleanupReason::ManualReset,
        },
        ms(11),
    );
    assert_eq!(s.view().cleanup_work, tp::CleanupWorkState::CleanupPending);

    // The online reversal extinguishes only the offline-candidate domain it
    // defines...
    s.accept(online(1, 3), ms(20));
    assert!(s.view().offline_candidate.is_none());
    assert_eq!(s.view().network_path, tp::NetworkPathState::Online);
    // ...and must not touch the independently-pending cleanup obligation.
    assert_eq!(s.view().cleanup_work, tp::CleanupWorkState::CleanupPending);
    assert_eq!(s.composite_action(), Some(tp::Action::Cleanup));
}

// -- preemption records a cancel reason (invariant 27 slot release) ---------

#[test]
fn inv27_cleanup_preempts_a_running_probe() {
    let mut s = ungated();
    s.accept(
        tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Probe,
            reason: tp::RecoveryRequestReason::ManualReconnect,
        },
        ms(1),
    );
    let started = s.maybe_start_effect(ms(2)).expect("probe should start");
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
