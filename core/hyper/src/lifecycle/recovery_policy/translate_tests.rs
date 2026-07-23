//! Reducer-level coverage: at least one test per translation-table row, the
//! composite decision table, and the derived send projection.

use super::super::classification::{FixedEntropy, ParkCause};
use super::*;

fn config() -> PolicyConfig {
    PolicyConfig::defaults()
}

/// Translate at `t = 0` with a benign deterministic entropy source.
fn tr(view: &View, input: Input) -> Decision {
    tr_at(view, input, Duration::ZERO)
}

fn tr_at(view: &View, input: Input, now: Duration) -> Decision {
    let mut entropy = FixedEntropy::constant(0.0);
    translate(view, &input, now, &config(), &mut entropy)
}

fn foreground() -> Input {
    Input::AppEnteredForeground {
        observed_background_duration: None,
    }
}

fn has(d: &Decision, input: MachineInput) -> bool {
    d.machine_inputs.contains(&input)
}

fn recovery_effect(action_id: u64, kind: EffectKind, revision: Revision) -> EffectContext {
    EffectContext {
        action_id,
        kind,
        captured_revision: revision,
        cancel_reason: None,
    }
}

// ---------------------------------------------------------------------------
// AppEnteredForeground
// ---------------------------------------------------------------------------

#[test]
fn foreground_while_logged_out_only_updates_phase() {
    let mut view = View::initial();
    view.recovery_mode = RecoveryModeState::LoggedOut;
    view.app_phase = AppPhaseState::Background;
    let d = tr(&view, foreground());
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert_eq!(
        d.machine_inputs,
        vec![MachineInput::AppPhase(AppPhaseInput::EnterForeground)]
    );
}

#[test]
fn foreground_from_background_derives_probe_or_reconnect_by_duration() {
    let mut view = View::initial();
    view.app_phase = AppPhaseState::Background;
    view.background_entered_at = Some(Duration::ZERO);

    // Short background -> probe.
    let d = tr_at(&view, foreground(), Duration::from_secs(1));
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestProbe)
    ));
    assert!(d.gate_triggers.contains(&GateTrigger::Wake {
        domain: RetryDomain::Recovery
    }));

    // Long background -> reconnect.
    let d = tr_at(&view, foreground(), Duration::from_secs(120));
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestReconnect)
    ));
}

#[test]
fn foreground_first_authoritative_phase_wakes_backing_off() {
    let mut view = View::initial();
    view.app_phase = AppPhaseState::Unknown;
    let d = tr(&view, foreground());
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(has(
        &d,
        MachineInput::AppPhase(AppPhaseInput::EnterForeground)
    ));
    assert!(
        !d.machine_inputs
            .iter()
            .any(|m| matches!(m, MachineInput::RecoveryIntent(_)))
    );
    assert!(d.gate_triggers.contains(&GateTrigger::Wake {
        domain: RetryDomain::Recovery
    }));
}

#[test]
fn foreground_already_foreground_is_a_no_op() {
    let mut view = View::initial();
    view.app_phase = AppPhaseState::Foreground;
    let d = tr(&view, foreground());
    assert_eq!(d, Decision::none());
}

// ---------------------------------------------------------------------------
// AppEnteredBackground
// ---------------------------------------------------------------------------

#[test]
fn background_transitions_and_advances() {
    let mut view = View::initial();
    view.app_phase = AppPhaseState::Foreground;
    let d = tr(&view, Input::AppEnteredBackground);
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert_eq!(
        d.machine_inputs,
        vec![MachineInput::AppPhase(AppPhaseInput::EnterBackground)]
    );
}

#[test]
fn background_emits_suppress_auto_reconnect_directive() {
    // Entering the background derives SuppressAutoReconnect through translation
    // rather than pre-translation event sniffing.
    let mut view = View::initial();
    view.app_phase = AppPhaseState::Foreground;
    let d = tr(&view, Input::AppEnteredBackground);
    assert!(
        d.signals
            .contains(&SignalingDirective::SuppressAutoReconnect)
    );
}

#[test]
fn short_foreground_emits_resume_and_long_foreground_emits_suppress() {
    let mut view = View::initial();
    view.app_phase = AppPhaseState::Background;
    view.background_entered_at = Some(Duration::ZERO);

    // Short background -> probe path re-enables automatic reconnect.
    let d = tr_at(&view, foreground(), Duration::from_secs(1));
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestProbe)
    ));
    assert!(d.signals.contains(&SignalingDirective::ResumeAutoReconnect));

    // Long background -> reconnect path keeps stale automatic reconnect suppressed.
    let d = tr_at(&view, foreground(), Duration::from_secs(120));
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestReconnect)
    ));
    assert!(
        d.signals
            .contains(&SignalingDirective::SuppressAutoReconnect)
    );
}

#[test]
fn short_foreground_coalesces_probe_while_restore_is_running() {
    let mut view = running_recovery_view(EffectKind::Restore, RecoveryStrength::Restore);
    view.app_phase = AppPhaseState::Background;
    view.background_entered_at = Some(Duration::ZERO);

    let d = tr_at(&view, foreground(), Duration::from_secs(1));

    assert!(has(
        &d,
        MachineInput::AppPhase(AppPhaseInput::EnterForeground)
    ));
    assert!(!has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestProbe)
    ));
    assert!(d.signals.contains(&SignalingDirective::ResumeAutoReconnect));
    assert!(d.cancels.is_empty());
}

#[test]
fn long_foreground_preempts_restore_and_fences_its_signaling_attempt() {
    let mut view = running_recovery_view(EffectKind::Restore, RecoveryStrength::Restore);
    view.app_phase = AppPhaseState::Background;
    view.background_entered_at = Some(Duration::ZERO);

    let d = tr_at(&view, foreground(), Duration::from_secs(120));

    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestReconnect)
    ));
    assert!(d.cancels.contains(&CancelReason::PreemptedByStronger));
    assert!(
        d.signals
            .contains(&SignalingDirective::InvalidateConnectionAttempts),
        "the old Restore must lose signaling commit rights before cancellation"
    );
}

#[test]
fn foreground_uses_platform_background_duration_when_delivery_was_delayed() {
    let mut view = View::initial();
    view.app_phase = AppPhaseState::Background;
    view.background_entered_at = Some(Duration::from_secs(99));

    let d = tr_at(
        &view,
        Input::AppEnteredForeground {
            observed_background_duration: Some(Duration::from_secs(60)),
        },
        Duration::from_secs(100),
    );

    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestReconnect)
    ));
    assert!(
        d.signals
            .contains(&SignalingDirective::SuppressAutoReconnect)
    );
}

#[test]
fn foreground_between_legacy_and_configured_thresholds_probes_and_resumes() {
    // Pins the single 60 s `background_reconnect_after` boundary: a stay in
    // the 30..60 s band is a short background. The legacy pre-translate hook
    // suppressed automatic reconnect at 30 s while intent selection already
    // used 60 s; the translation layer resolves that contradiction in favor
    // of the configured 60 s default, so 45 s must probe and resume.
    let mut view = View::initial();
    view.app_phase = AppPhaseState::Background;
    view.background_entered_at = Some(Duration::ZERO);

    let d = tr_at(&view, foreground(), Duration::from_secs(45));
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestProbe)
    ));
    assert!(d.signals.contains(&SignalingDirective::ResumeAutoReconnect));
    assert!(
        !d.signals
            .contains(&SignalingDirective::SuppressAutoReconnect)
    );
}

#[test]
fn foreground_reconnect_boundary_is_exact_at_sixty_seconds() {
    let mut view = View::initial();
    view.app_phase = AppPhaseState::Background;
    view.background_entered_at = Some(Duration::ZERO);

    for elapsed_ms in [59_999, 60_000, 60_001] {
        let d = tr_at(&view, foreground(), Duration::from_millis(elapsed_ms));
        let is_long_background = elapsed_ms >= 60_000;

        assert!(
            has(
                &d,
                MachineInput::RecoveryIntent(if is_long_background {
                    RecoveryIntentInput::RequestReconnect
                } else {
                    RecoveryIntentInput::RequestProbe
                })
            ),
            "unexpected recovery intent at {elapsed_ms} ms"
        );
        assert!(
            d.signals.contains(if is_long_background {
                &SignalingDirective::SuppressAutoReconnect
            } else {
                &SignalingDirective::ResumeAutoReconnect
            }),
            "unexpected signaling directive at {elapsed_ms} ms"
        );
    }
}

#[test]
fn background_already_background_is_a_no_op() {
    let mut view = View::initial();
    view.app_phase = AppPhaseState::Background;
    assert_eq!(tr(&view, Input::AppEnteredBackground), Decision::none());
}

// ---------------------------------------------------------------------------
// SessionActivated
// ---------------------------------------------------------------------------

#[test]
fn session_activated_newer_commits_and_derives_restore() {
    let mut view = View::initial();
    view.recovery_mode = RecoveryModeState::LoggedOut;
    view.committed_session_generation = Some(1);
    let d = tr(
        &view,
        Input::SessionActivated {
            session_generation: 2,
        },
    );
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(has(
        &d,
        MachineInput::RecoveryMode(RecoveryModeInput::SessionActivated)
    ));
    // No live signaling generation exists -> restoration derived.
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestRestore)
    ));
    assert!(d.gate_triggers.contains(&GateTrigger::ClearMask {
        domain: RetryDomain::Recovery,
        trigger: TriggerClass::SessionActivated
    }));
}

#[test]
fn session_activated_not_newer_is_ignored() {
    let mut view = View::initial();
    view.committed_session_generation = Some(5);
    assert_eq!(
        tr(
            &view,
            Input::SessionActivated {
                session_generation: 5
            }
        ),
        Decision::none()
    );
}

#[test]
fn session_activated_supersedes_existing_intent() {
    let mut view = View::initial();
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    view.recovery_record = Some(PendingRecord::recovery(3, RecoveryStrength::Reconnect));
    let d = tr(
        &view,
        Input::SessionActivated {
            session_generation: 1,
        },
    );
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::SupersedeRecovery)
    ));
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestRestore)
    ));
}

#[test]
fn session_activated_skips_restore_when_live_generation_outside_teardown() {
    let mut view = View::initial();
    view.live_signaling_generation = Some(9);
    let d = tr(
        &view,
        Input::SessionActivated {
            session_generation: 1,
        },
    );
    assert!(!has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestRestore)
    ));
}

#[test]
fn session_activated_derives_restore_when_live_generation_is_inside_teardown() {
    let mut view = View::initial();
    view.live_signaling_generation = Some(9);
    view.teardown_scope_generations.insert(9);
    let d = tr(
        &view,
        Input::SessionActivated {
            session_generation: 1,
        },
    );
    // A generation circled by pending teardown does not count as live.
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestRestore)
    ));
}

// ---------------------------------------------------------------------------
// NetworkSnapshot
// ---------------------------------------------------------------------------

fn snapshot(epoch: u64, sequence: u64, path: SemanticPath, route: u64) -> Input {
    Input::NetworkSnapshot {
        source_epoch: epoch,
        sequence,
        semantic_path: path,
        route_fingerprint: route,
    }
}

#[test]
fn snapshot_stale_epoch_or_sequence_is_discarded() {
    let mut view = View::initial();
    view.last_snapshot = Some(AcceptedSnapshot {
        source_epoch: 2,
        sequence: 5,
        semantic_path: SemanticPath::Online,
        route_fingerprint: 1,
    });
    // Older epoch.
    assert_eq!(
        tr(&view, snapshot(1, 99, SemanticPath::Offline, 2)),
        Decision::none()
    );
    // Same epoch, not strictly newer sequence.
    assert_eq!(
        tr(&view, snapshot(2, 5, SemanticPath::Offline, 2)),
        Decision::none()
    );
}

#[test]
fn snapshot_structural_duplicate_is_discarded() {
    let mut view = View::initial();
    view.last_snapshot = Some(AcceptedSnapshot {
        source_epoch: 1,
        sequence: 1,
        semantic_path: SemanticPath::Online,
        route_fingerprint: 7,
    });
    // Newer sequence but identical semantics.
    assert_eq!(
        tr(&view, snapshot(1, 2, SemanticPath::Online, 7)),
        Decision::none()
    );
}

#[test]
fn snapshot_to_online_observes_and_derives_recovery() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::Offline;
    let d = tr(&view, snapshot(1, 1, SemanticPath::Online, 1));
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(has(
        &d,
        MachineInput::NetworkPath(NetworkPathInput::ObserveOnline)
    ));
    assert!(d.timers.contains(&TimerDirective::Cancel {
        id: TimerId::OfflineCandidate
    }));
    // No live signaling generation -> restore derived (not probe).
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestRestore)
    ));
}

#[test]
fn snapshot_online_derives_probe_when_live_generation_exists() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::OfflineCandidate;
    view.live_signaling_generation = Some(4);
    let d = tr(&view, snapshot(1, 1, SemanticPath::Online, 1));
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestProbe)
    ));
}

#[test]
fn snapshot_online_ignores_teardown_scoped_live_generation() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::Offline;
    view.offline_work = OfflineWorkState::DisconnectPending;
    view.execution = ExecutionState::Disconnecting;
    view.live_signaling_generation = Some(4);
    view.teardown_scope_generations.insert(4);

    let d = tr(&view, snapshot(1, 1, SemanticPath::Online, 1));

    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestRestore)
    ));
    assert!(!has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestProbe)
    ));
}

#[test]
fn snapshot_online_supersedes_pending_disconnect() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::Offline;
    view.offline_work = OfflineWorkState::DisconnectPending;
    view.offline_record = Some(PendingRecord::teardown(1));
    let d = tr(&view, snapshot(1, 1, SemanticPath::Online, 1));
    assert!(has(
        &d,
        MachineInput::OfflineWork(OfflineWorkInput::SupersedeDisconnect)
    ));
}

#[test]
fn snapshot_online_to_online_material_route_only_triggers() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::Online;
    view.live_signaling_generation = Some(7);
    view.last_snapshot = Some(AcceptedSnapshot {
        source_epoch: 1,
        sequence: 1,
        semantic_path: SemanticPath::Online,
        route_fingerprint: 1,
    });
    let d = tr(&view, snapshot(1, 2, SemanticPath::Online, 2));
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(d.machine_inputs.is_empty());
    assert!(d.gate_triggers.contains(&GateTrigger::Wake {
        domain: RetryDomain::Recovery
    }));
}

#[test]
fn snapshot_online_route_during_cleanup_retains_post_cleanup_restore() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::Online;
    view.last_snapshot = Some(AcceptedSnapshot {
        source_epoch: 1,
        sequence: 1,
        semantic_path: SemanticPath::Online,
        route_fingerprint: 1,
    });
    view.cleanup_work = CleanupWorkState::CleanupPending;
    view.execution = ExecutionState::Cleaning;
    view.live_signaling_generation = Some(7);
    view.teardown_scope_generations.insert(7);

    let d = tr(&view, snapshot(1, 2, SemanticPath::Online, 2));

    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestRestore)
    ));
    assert!(d.gate_triggers.contains(&GateTrigger::Wake {
        domain: RetryDomain::Recovery
    }));
}

#[test]
fn snapshot_to_offline_arms_candidate() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::Online;
    view.last_snapshot = Some(AcceptedSnapshot {
        source_epoch: 1,
        sequence: 1,
        semantic_path: SemanticPath::Online,
        route_fingerprint: 1,
    });
    let d = tr_at(
        &view,
        snapshot(1, 2, SemanticPath::Offline, 1),
        Duration::from_secs(1),
    );
    assert!(has(
        &d,
        MachineInput::NetworkPath(NetworkPathInput::ObserveOffline)
    ));
    assert!(d.timers.contains(&TimerDirective::Arm {
        id: TimerId::OfflineCandidate,
        category: TimerCategory::BusinessHysteresis,
        deadline: Duration::from_secs(1) + config().offline_grace,
    }));
}

#[test]
fn snapshot_to_unknown_invalidates_candidate() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::OfflineCandidate;
    view.last_snapshot = Some(AcceptedSnapshot {
        source_epoch: 1,
        sequence: 1,
        semantic_path: SemanticPath::Offline,
        route_fingerprint: 1,
    });
    let d = tr(&view, snapshot(1, 2, SemanticPath::Unknown, 1));
    assert!(has(
        &d,
        MachineInput::NetworkPath(NetworkPathInput::ObserveUnknown)
    ));
    assert!(d.timers.contains(&TimerDirective::Cancel {
        id: TimerId::OfflineCandidate
    }));
}

// ---------------------------------------------------------------------------
// OfflineGraceExpired
// ---------------------------------------------------------------------------

#[test]
fn offline_grace_expiry_commits_and_requests_disconnect() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::OfflineCandidate;
    view.offline_candidate = Some(OfflineCandidate {
        candidate_id: 7,
        deadline: Duration::from_millis(400),
    });
    let d = tr(&view, Input::OfflineGraceExpired { candidate_id: 7 });
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(has(
        &d,
        MachineInput::NetworkPath(NetworkPathInput::CommitOffline)
    ));
    assert!(has(
        &d,
        MachineInput::OfflineWork(OfflineWorkInput::RequestDisconnect)
    ));
    assert!(d.timers.iter().any(|t| matches!(
        t,
        TimerDirective::Arm {
            id: TimerId::TeardownOverall(TeardownDomain::OfflineDisconnect),
            ..
        }
    )));
}

#[test]
fn offline_grace_expiry_stale_candidate_is_ignored() {
    let mut view = View::initial();
    view.offline_candidate = Some(OfflineCandidate {
        candidate_id: 7,
        deadline: Duration::from_millis(400),
    });
    assert_eq!(
        tr(&view, Input::OfflineGraceExpired { candidate_id: 8 }),
        Decision::none()
    );
}

// ---------------------------------------------------------------------------
// RetryDeadlineExpired
// ---------------------------------------------------------------------------

#[test]
fn retry_deadline_expiry_wakes_matching_backing_off_record() {
    let mut view = View::initial();
    let mut rec = PendingRecord::recovery(4, RecoveryStrength::Restore);
    rec.gate = RetryGateState::BackingOff;
    rec.retry_id = 11;
    view.recovery_intent = RecoveryIntentState::RestorePending;
    view.recovery_record = Some(rec);
    let d = tr(
        &view,
        Input::RetryDeadlineExpired {
            domain: RetryDomain::Recovery,
            work_revision: 4,
            retry_id: 11,
        },
    );
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(has(
        &d,
        MachineInput::RetryGate {
            domain: RetryDomain::Recovery,
            input: RetryGateInput::RetryDeadlineExpired
        }
    ));
}

#[test]
fn retry_deadline_expiry_mismatch_is_ignored() {
    let mut view = View::initial();
    let mut rec = PendingRecord::recovery(4, RecoveryStrength::Restore);
    rec.gate = RetryGateState::BackingOff;
    rec.retry_id = 11;
    view.recovery_record = Some(rec);
    // Wrong retry_id.
    assert_eq!(
        tr(
            &view,
            Input::RetryDeadlineExpired {
                domain: RetryDomain::Recovery,
                work_revision: 4,
                retry_id: 99
            }
        ),
        Decision::none()
    );
}

// ---------------------------------------------------------------------------
// RecoveryRequested
// ---------------------------------------------------------------------------

fn recovery_requested(minimum: RecoveryStrength) -> Input {
    Input::RecoveryRequested {
        minimum,
        reason: RecoveryRequestReason::ManualReconnect,
    }
}

#[test]
fn recovery_requested_rejected_when_logged_out() {
    let mut view = View::initial();
    view.recovery_mode = RecoveryModeState::LoggedOut;
    let d = tr(&view, recovery_requested(RecoveryStrength::Reconnect));
    assert_eq!(d.revision, RevisionDirective::Unchanged);
    assert!(d.status.contains(&StatusRecord::RecoveryRejected {
        mode: RecoveryModeState::LoggedOut,
        reason: RejectReason::LoggedOutOrTerminating
    }));
}

#[test]
fn recovery_requested_idle_or_weaker_requests_stronger() {
    let view = View::initial();
    let d = tr(&view, recovery_requested(RecoveryStrength::Restore));
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestRestore)
    ));
}

#[test]
fn recovery_requested_stronger_than_pending_promotes() {
    let mut view = View::initial();
    view.recovery_intent = RecoveryIntentState::ProbePending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Probe));
    let d = tr(&view, recovery_requested(RecoveryStrength::Reconnect));
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestReconnect)
    ));
}

#[test]
fn recovery_requested_preempts_weaker_running_effect() {
    // A probe is running when a stronger reconnect is requested: the reconnect
    // intent is requested and the running probe is preempted so it starts once
    // the single-flight slot returns to Idle (RFC effect-preemption table).
    let mut view = View::initial();
    view.recovery_intent = RecoveryIntentState::ProbePending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Probe));
    view.execution = ExecutionState::Probing;
    view.effect = Some(recovery_effect(1, EffectKind::Probe, 1));
    let d = tr(&view, recovery_requested(RecoveryStrength::Reconnect));
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestReconnect)
    ));
    assert!(d.cancels.contains(&CancelReason::PreemptedByStronger));
    assert!(
        d.signals
            .contains(&SignalingDirective::InvalidateConnectionAttempts)
    );
}

#[test]
fn recovery_requested_does_not_preempt_equal_or_stronger_running_effect() {
    // A restore is running when a weaker probe is requested: the request is
    // coalesced and nothing is preempted.
    let mut view = View::initial();
    view.recovery_intent = RecoveryIntentState::RestorePending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Restore));
    view.execution = ExecutionState::Restoring;
    view.effect = Some(recovery_effect(1, EffectKind::Restore, 1));
    let d = tr(&view, recovery_requested(RecoveryStrength::Probe));
    assert!(d.cancels.is_empty());
}

#[test]
fn recovery_requested_coalesced_when_ready_and_at_least_as_strong() {
    let mut view = View::initial();
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Reconnect));
    assert_eq!(
        tr(&view, recovery_requested(RecoveryStrength::Restore)),
        Decision::none()
    );
}

#[test]
fn recovery_requested_backing_off_is_a_deliberate_retry() {
    let mut view = View::initial();
    let mut rec = PendingRecord::recovery(1, RecoveryStrength::Reconnect);
    rec.gate = RetryGateState::BackingOff;
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    view.recovery_record = Some(rec);
    let d = tr(&view, recovery_requested(RecoveryStrength::Reconnect));
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(d.gate_triggers.contains(&GateTrigger::Wake {
        domain: RetryDomain::Recovery
    }));
}

#[test]
fn recovery_requested_parked_clears_matching_or_is_rejected() {
    let mut view = View::initial();
    let mut rec = PendingRecord::recovery(1, RecoveryStrength::Reconnect);
    rec.gate = RetryGateState::Parked;
    rec.release_mask = ReleaseMask::single(ParkCause::InvariantViolation);
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    view.recovery_record = Some(rec);
    // An explicit command clears the invariant park entry.
    let d = tr(&view, recovery_requested(RecoveryStrength::Reconnect));
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(d.gate_triggers.contains(&GateTrigger::ClearMask {
        domain: RetryDomain::Recovery,
        trigger: TriggerClass::ExplicitCommand
    }));

    // An explicit-pause-only park is not cleared by an explicit command.
    let mut rec = PendingRecord::recovery(1, RecoveryStrength::Reconnect);
    rec.gate = RetryGateState::Parked;
    rec.release_mask = ReleaseMask::single(ParkCause::ExplicitPause);
    view.recovery_record = Some(rec);
    let d = tr(&view, recovery_requested(RecoveryStrength::Reconnect));
    assert_eq!(d.revision, RevisionDirective::Unchanged);
    assert!(d.status.contains(&StatusRecord::RecoveryRejected {
        mode: RecoveryModeState::Active,
        reason: RejectReason::ParkedNoClearingTrigger
    }));
}

// ---------------------------------------------------------------------------
// RecoveryPause / RecoveryResume
// ---------------------------------------------------------------------------

#[test]
fn pause_in_flight_requests_cancellation_without_moving_gate() {
    let mut view = View::initial();
    view.execution = ExecutionState::Reconnecting;
    view.effect = Some(recovery_effect(1, EffectKind::Reconnect, 1));
    let d = tr(
        &view,
        Input::RecoveryPauseRequested {
            scope: RecoveryScope::AllRecovery,
        },
    );
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(d.cancels.contains(&CancelReason::Pause));
    assert!(d.machine_inputs.is_empty());
}

#[test]
fn pause_idle_record_parks_and_cancels_deadline() {
    let mut view = View::initial();
    view.recovery_intent = RecoveryIntentState::RestorePending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Restore));
    let d = tr(
        &view,
        Input::RecoveryPauseRequested {
            scope: RecoveryScope::AllRecovery,
        },
    );
    assert!(has(
        &d,
        MachineInput::RetryGate {
            domain: RetryDomain::Recovery,
            input: RetryGateInput::ExplicitPause
        }
    ));
    assert!(d.parks.contains(&ParkDirective {
        domain: RetryDomain::Recovery,
        release_mask: ReleaseMask::single(ParkCause::ExplicitPause)
    }));
    assert!(d.timers.contains(&TimerDirective::Cancel {
        id: TimerId::FailureBackoff(RetryDomain::Recovery)
    }));
}

#[test]
fn resume_clears_explicit_pause_entries() {
    let mut view = View::initial();
    let mut rec = PendingRecord::recovery(1, RecoveryStrength::Restore);
    rec.gate = RetryGateState::Parked;
    rec.release_mask = ReleaseMask::single(ParkCause::ExplicitPause);
    view.recovery_record = Some(rec);
    let d = tr(
        &view,
        Input::RecoveryResumeRequested {
            scope: RecoveryScope::AllRecovery,
        },
    );
    assert!(d.gate_triggers.contains(&GateTrigger::ClearMask {
        domain: RetryDomain::Recovery,
        trigger: TriggerClass::ExplicitResume
    }));
}

#[test]
fn resume_without_pause_entry_is_a_no_op() {
    let view = View::initial();
    assert_eq!(
        tr(
            &view,
            Input::RecoveryResumeRequested {
                scope: RecoveryScope::AllRecovery
            }
        ),
        Decision::none()
    );
}

// ---------------------------------------------------------------------------
// ConfigurationChanged
// ---------------------------------------------------------------------------

#[test]
fn configuration_changed_clears_config_parks_and_is_material() {
    let view = View::initial();
    let d = tr(
        &view,
        Input::ConfigurationChanged {
            scope: ConfigScope::All,
        },
    );
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(d.gate_triggers.contains(&GateTrigger::ClearMask {
        domain: RetryDomain::Recovery,
        trigger: TriggerClass::ConfigurationChanged
    }));
    assert!(d.gate_triggers.contains(&GateTrigger::Wake {
        domain: RetryDomain::Recovery
    }));
}

// ---------------------------------------------------------------------------
// CleanupRequested
// ---------------------------------------------------------------------------

#[test]
fn cleanup_user_logout_enters_logged_out_and_requests_cleanup() {
    let view = View::initial();
    let d = tr(
        &view,
        Input::CleanupRequested {
            reason: CleanupReason::UserLogout,
        },
    );
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(has(
        &d,
        MachineInput::RecoveryMode(RecoveryModeInput::UserLoggedOut)
    ));
    assert!(has(
        &d,
        MachineInput::CleanupWork(CleanupWorkInput::RequestCleanup)
    ));
    assert!(d.timers.iter().any(|t| matches!(
        t,
        TimerDirective::Arm {
            id: TimerId::TeardownOverall(TeardownDomain::Cleanup),
            ..
        }
    )));
}

#[test]
fn cleanup_app_terminating_arms_shutdown_deadline() {
    let view = View::initial();
    let d = tr(
        &view,
        Input::CleanupRequested {
            reason: CleanupReason::AppTerminating,
        },
    );
    assert!(has(
        &d,
        MachineInput::RecoveryMode(RecoveryModeInput::AppTerminating)
    ));
    assert!(d.timers.iter().any(|t| matches!(
        t,
        TimerDirective::Arm {
            id: TimerId::ShutdownOverall,
            ..
        }
    )));
}

#[test]
fn cleanup_manual_reset_leaves_mode_unchanged() {
    let view = View::initial();
    let d = tr(
        &view,
        Input::CleanupRequested {
            reason: CleanupReason::ManualReset,
        },
    );
    assert!(
        !d.machine_inputs
            .iter()
            .any(|m| matches!(m, MachineInput::RecoveryMode(_)))
    );
    assert!(has(
        &d,
        MachineInput::CleanupWork(CleanupWorkInput::RequestCleanup)
    ));
}

#[test]
fn cleanup_stale_connection_leaves_mode_unchanged() {
    let view = View::initial();
    let d = tr(
        &view,
        Input::CleanupRequested {
            reason: CleanupReason::StaleConnectionSuspected,
        },
    );
    assert!(
        !d.machine_inputs
            .iter()
            .any(|m| matches!(m, MachineInput::RecoveryMode(_)))
    );
    assert!(has(
        &d,
        MachineInput::CleanupWork(CleanupWorkInput::RequestCleanup)
    ));
}

#[test]
fn cleanup_supersedes_and_preempts_and_commits_candidate() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::OfflineCandidate;
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Reconnect));
    view.offline_work = OfflineWorkState::DisconnectPending;
    view.offline_record = Some(PendingRecord::teardown(1));
    view.execution = ExecutionState::Reconnecting;
    view.effect = Some(recovery_effect(2, EffectKind::Reconnect, 1));
    let d = tr(
        &view,
        Input::CleanupRequested {
            reason: CleanupReason::UserLogout,
        },
    );
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::SupersedeRecovery)
    ));
    assert!(has(
        &d,
        MachineInput::OfflineWork(OfflineWorkInput::SupersedeDisconnect)
    ));
    assert!(has(
        &d,
        MachineInput::NetworkPath(NetworkPathInput::CommitOffline)
    ));
    assert!(d.cancels.contains(&CancelReason::PreemptedByCleanup));
    assert!(d.timers.contains(&TimerDirective::Cancel {
        id: TimerId::OfflineCandidate
    }));
}

#[test]
fn shutdown_requested_matches_app_terminating_cleanup() {
    let view = View::initial();
    let a = tr(&view, Input::ShutdownRequested);
    let b = tr(
        &view,
        Input::CleanupRequested {
            reason: CleanupReason::AppTerminating,
        },
    );
    assert_eq!(a, b);
}

// ---------------------------------------------------------------------------
// Bootstrap / Shutdown / Teardown deadlines
// ---------------------------------------------------------------------------

#[test]
fn bootstrap_deadline_reports_error_only_when_gated_and_unknown() {
    let mut view = View::initial();
    view.profile = LifecycleProfile::Gated;
    view.app_phase = AppPhaseState::Unknown;
    let d = tr(&view, Input::BootstrapPhaseDeadlineExpired);
    assert_eq!(d.revision, RevisionDirective::Unchanged);
    assert!(d.status.contains(&StatusRecord::BootstrapDeadlineElapsed));

    // Ungated -> nothing.
    let view = View::initial();
    assert_eq!(
        tr(&view, Input::BootstrapPhaseDeadlineExpired),
        Decision::none()
    );
}

#[test]
fn shutdown_deadline_aborts_when_terminating_and_matching() {
    let mut view = View::initial();
    view.recovery_mode = RecoveryModeState::Terminating;
    view.execution = ExecutionState::Cleaning;
    view.effect = Some(recovery_effect(1, EffectKind::Cleanup, 1));
    view.shutdown_deadline = Some(ShutdownDeadline {
        deadline_id: 3,
        deadline: Duration::from_secs(10),
    });
    let d = tr(&view, Input::ShutdownDeadlineExpired { deadline_id: 3 });
    assert!(d.status.contains(&StatusRecord::ShutdownAbandon));
    assert!(d.cancels.contains(&CancelReason::Shutdown));
    // The supervisor ends unconditionally: the shell stops its reconcile loop.
    assert!(d.terminate);

    // Stale id -> nothing.
    let stale = tr(&view, Input::ShutdownDeadlineExpired { deadline_id: 4 });
    assert_eq!(stale, Decision::none());
    assert!(!stale.terminate);
}

#[test]
fn shutdown_deadline_detaches_pending_teardown_and_terminates() {
    // A cleanup obligation is still pending (backing off) when the shutdown
    // overall deadline fires: it must be detached with Abandoned semantics so it
    // cannot restart, and the supervisor must terminate rather than reconcile
    // cleanup again.
    let mut view = View::initial();
    view.recovery_mode = RecoveryModeState::Terminating;
    view.cleanup_work = CleanupWorkState::CleanupPending;
    let mut rec = PendingRecord::teardown(1);
    rec.gate = RetryGateState::BackingOff;
    view.cleanup_record = Some(rec);
    view.offline_work = OfflineWorkState::DisconnectPending;
    view.offline_record = Some(PendingRecord::teardown(1));
    view.shutdown_deadline = Some(ShutdownDeadline {
        deadline_id: 7,
        deadline: Duration::from_secs(10),
    });
    let d = tr(&view, Input::ShutdownDeadlineExpired { deadline_id: 7 });
    assert!(has(
        &d,
        MachineInput::CleanupWork(CleanupWorkInput::CompleteCleanup)
    ));
    assert!(has(
        &d,
        MachineInput::OfflineWork(OfflineWorkInput::CompleteDisconnect)
    ));
    assert!(d.status.contains(&StatusRecord::ShutdownAbandon));
    assert!(d.terminate);
}

#[test]
fn teardown_deadline_abandons_backing_off_obligation() {
    let mut view = View::initial();
    let mut rec = PendingRecord::teardown(1);
    rec.gate = RetryGateState::BackingOff;
    view.cleanup_work = CleanupWorkState::CleanupPending;
    view.cleanup_record = Some(rec);
    view.cleanup_teardown = Some(TeardownDeadline {
        deadline_id: 5,
        deadline: Duration::from_secs(10),
    });
    let d = tr(
        &view,
        Input::TeardownDeadlineExpired {
            domain: TeardownDomain::Cleanup,
            deadline_id: 5,
        },
    );
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(has(
        &d,
        MachineInput::CleanupWork(CleanupWorkInput::CompleteCleanup)
    ));
}

#[test]
fn teardown_deadline_ignored_while_effect_running() {
    let mut view = View::initial();
    let mut rec = PendingRecord::teardown(1);
    rec.gate = RetryGateState::BackingOff;
    view.cleanup_work = CleanupWorkState::CleanupPending;
    view.cleanup_record = Some(rec);
    view.execution = ExecutionState::Cleaning;
    view.cleanup_teardown = Some(TeardownDeadline {
        deadline_id: 5,
        deadline: Duration::from_secs(10),
    });
    assert_eq!(
        tr(
            &view,
            Input::TeardownDeadlineExpired {
                domain: TeardownDomain::Cleanup,
                deadline_id: 5
            }
        ),
        Decision::none()
    );
}

// ---------------------------------------------------------------------------
// Signaling generation facts
// ---------------------------------------------------------------------------

#[test]
fn signaling_committed_current_effect_is_covered_output() {
    let mut view = View::initial();
    view.effect = Some(recovery_effect(8, EffectKind::Reconnect, 2));
    let d = tr(
        &view,
        Input::SignalingGenerationCommitted {
            generation: 5,
            origin: SignalingOrigin::CurrentEffect { action_id: 8 },
        },
    );
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(d.gate_triggers.is_empty());
}

#[test]
fn signaling_committed_external_newer_is_material() {
    let mut view = View::initial();
    view.live_signaling_generation = Some(3);
    let d = tr(
        &view,
        Input::SignalingGenerationCommitted {
            generation: 4,
            origin: SignalingOrigin::External,
        },
    );
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(d.gate_triggers.contains(&GateTrigger::Wake {
        domain: RetryDomain::Recovery
    }));
    // Not newer -> nothing.
    let d = tr(
        &view,
        Input::SignalingGenerationCommitted {
            generation: 3,
            origin: SignalingOrigin::External,
        },
    );
    assert_eq!(d, Decision::none());
}

#[test]
fn signaling_lost_derives_restore_when_active_and_no_cleanup() {
    let mut view = View::initial();
    view.live_signaling_generation = Some(6);
    let d = tr(
        &view,
        Input::SignalingGenerationLost {
            generation: 6,
            cause: SignalingLostCause::Disconnected,
        },
    );
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestRestore)
    ));
    assert!(d.gate_triggers.contains(&GateTrigger::Wake {
        domain: RetryDomain::Recovery
    }));
}

#[test]
fn signaling_lost_by_current_reconnect_is_covered_output() {
    let mut view = View::initial();
    view.live_signaling_generation = Some(6);
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    view.recovery_record = Some(PendingRecord::recovery(2, RecoveryStrength::Reconnect));
    view.execution = ExecutionState::Reconnecting;
    view.effect = Some(recovery_effect(8, EffectKind::Reconnect, 2));

    let d = tr(
        &view,
        Input::SignalingGenerationLost {
            generation: 6,
            cause: SignalingLostCause::Disconnected,
        },
    );

    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(!has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestRestore)
    ));
    assert!(d.gate_triggers.is_empty());
}

#[test]
fn signaling_remote_reset_during_reconnect_remains_material() {
    let mut view = View::initial();
    view.live_signaling_generation = Some(6);
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    view.recovery_record = Some(PendingRecord::recovery(2, RecoveryStrength::Reconnect));
    view.execution = ExecutionState::Reconnecting;
    view.effect = Some(recovery_effect(8, EffectKind::Reconnect, 2));

    let d = tr(
        &view,
        Input::SignalingGenerationLost {
            generation: 6,
            cause: SignalingLostCause::RemoteReset,
        },
    );

    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestRestore)
    ));
}

#[test]
fn signaling_lost_absorbed_by_cleanup_derives_no_restore() {
    let mut view = View::initial();
    view.live_signaling_generation = Some(6);
    view.cleanup_work = CleanupWorkState::CleanupPending;
    let d = tr(
        &view,
        Input::SignalingGenerationLost {
            generation: 6,
            cause: SignalingLostCause::Disconnected,
        },
    );
    assert_eq!(d.revision, RevisionDirective::Advances);
    assert!(!has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestRestore)
    ));
}

#[test]
fn signaling_lost_stale_generation_is_ignored() {
    let mut view = View::initial();
    view.live_signaling_generation = Some(6);
    assert_eq!(
        tr(
            &view,
            Input::SignalingGenerationLost {
                generation: 5,
                cause: SignalingLostCause::Superseded
            }
        ),
        Decision::none()
    );
}

// ---------------------------------------------------------------------------
// EffectCompleted
// ---------------------------------------------------------------------------

fn running_recovery_view(kind: EffectKind, strength: RecoveryStrength) -> View {
    let mut view = View::initial();
    view.recovery_intent = match strength {
        RecoveryStrength::Probe => RecoveryIntentState::ProbePending,
        RecoveryStrength::Restore => RecoveryIntentState::RestorePending,
        RecoveryStrength::Reconnect => RecoveryIntentState::ReconnectPending,
    };
    view.recovery_record = Some(PendingRecord::recovery(1, strength));
    view.execution = match kind {
        EffectKind::Probe => ExecutionState::Probing,
        EffectKind::Restore => ExecutionState::Restoring,
        EffectKind::Reconnect => ExecutionState::Reconnecting,
        EffectKind::Cleanup => ExecutionState::Cleaning,
        EffectKind::ConfirmedOfflineDisconnect => ExecutionState::Disconnecting,
    };
    view.effect = Some(recovery_effect(1, kind, 1));
    view
}

#[test]
fn effect_completed_mismatch_is_discarded() {
    let view = running_recovery_view(EffectKind::Reconnect, RecoveryStrength::Reconnect);
    let d = tr(
        &view,
        Input::EffectCompleted {
            action_id: 99,
            kind: EffectKind::Reconnect,
            policy_revision: 1,
            outcome: EffectOutcome::Succeeded,
        },
    );
    assert_eq!(d, Decision::none());
}

#[test]
fn effect_completed_success_acknowledges_covered_intent() {
    let view = running_recovery_view(EffectKind::Restore, RecoveryStrength::Restore);
    let d = tr(
        &view,
        Input::EffectCompleted {
            action_id: 1,
            kind: EffectKind::Restore,
            policy_revision: 1,
            outcome: EffectOutcome::Succeeded,
        },
    );
    assert_eq!(d.revision, RevisionDirective::Unchanged);
    assert!(has(&d, MachineInput::Execution(ExecutionInput::Succeeded)));
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::CompleteRestore)
    ));
}

#[test]
fn effect_completed_failure_retry_backs_off() {
    let view = running_recovery_view(EffectKind::Reconnect, RecoveryStrength::Reconnect);
    let d = tr(
        &view,
        Input::EffectCompleted {
            action_id: 1,
            kind: EffectKind::Reconnect,
            policy_revision: 1,
            outcome: EffectOutcome::Failed {
                diagnosis: EffectDiagnosis::PathUnreachable {
                    stage: "connect".into(),
                },
            },
        },
    );
    assert!(has(&d, MachineInput::Execution(ExecutionInput::Failed)));
    assert!(has(
        &d,
        MachineInput::RetryGate {
            domain: RetryDomain::Recovery,
            input: RetryGateInput::RetryableFailure
        }
    ));
    assert!(d.timers.iter().any(|t| matches!(
        t,
        TimerDirective::Arm {
            id: TimerId::FailureBackoff(RetryDomain::Recovery),
            ..
        }
    )));
}

#[test]
fn effect_completed_failure_retry_honors_retry_after_floor() {
    let view = running_recovery_view(EffectKind::Reconnect, RecoveryStrength::Reconnect);
    // Overloaded with a large retry_after; jitter must not pierce the floor.
    let mut entropy = FixedEntropy::constant(1.0);
    let d = translate(
        &view,
        &Input::EffectCompleted {
            action_id: 1,
            kind: EffectKind::Reconnect,
            policy_revision: 1,
            outcome: EffectOutcome::Failed {
                diagnosis: EffectDiagnosis::Overloaded {
                    retry_after: Duration::from_secs(45),
                },
            },
        },
        Duration::ZERO,
        &config(),
        &mut entropy,
    );
    let armed = d
        .timers
        .iter()
        .find_map(|t| match t {
            TimerDirective::Arm {
                id: TimerId::FailureBackoff(_),
                deadline,
                ..
            } => Some(*deadline),
            _ => None,
        })
        .expect("backoff armed");
    assert_eq!(armed, Duration::from_secs(45));
}

#[test]
fn effect_completed_failure_escalates_probe_to_reconnect() {
    let view = running_recovery_view(EffectKind::Probe, RecoveryStrength::Probe);
    let d = tr(
        &view,
        Input::EffectCompleted {
            action_id: 1,
            kind: EffectKind::Probe,
            policy_revision: 1,
            outcome: EffectOutcome::Failed {
                diagnosis: EffectDiagnosis::GenerationDead { generation: 2 },
            },
        },
    );
    assert!(has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestReconnect)
    ));
    // No completion input for the replaced probe.
    assert!(!has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::CompleteProbe)
    ));
}

#[test]
fn effect_completed_failure_parks_on_precondition() {
    let view = running_recovery_view(EffectKind::Reconnect, RecoveryStrength::Reconnect);
    let d = tr(
        &view,
        Input::EffectCompleted {
            action_id: 1,
            kind: EffectKind::Reconnect,
            policy_revision: 1,
            outcome: EffectOutcome::Failed {
                diagnosis: EffectDiagnosis::AuthRejected {
                    kind: "expired".into(),
                },
            },
        },
    );
    assert!(has(
        &d,
        MachineInput::RetryGate {
            domain: RetryDomain::Recovery,
            input: RetryGateInput::TerminalFailure
        }
    ));
    assert!(d.parks.contains(&ParkDirective {
        domain: RetryDomain::Recovery,
        release_mask: ReleaseMask::single(ParkCause::AuthRejected)
    }));
}

#[test]
fn effect_completed_teardown_abandons_under_terminating_invariant() {
    let mut view = View::initial();
    view.recovery_mode = RecoveryModeState::Terminating;
    view.cleanup_work = CleanupWorkState::CleanupPending;
    view.cleanup_record = Some(PendingRecord::teardown(1));
    view.execution = ExecutionState::Cleaning;
    view.effect = Some(recovery_effect(1, EffectKind::Cleanup, 1));
    let d = tr(
        &view,
        Input::EffectCompleted {
            action_id: 1,
            kind: EffectKind::Cleanup,
            policy_revision: 1,
            outcome: EffectOutcome::Failed {
                diagnosis: EffectDiagnosis::InvariantViolation {
                    detail: "broken".into(),
                },
            },
        },
    );
    assert!(has(
        &d,
        MachineInput::CleanupWork(CleanupWorkInput::CompleteCleanup)
    ));
}

#[test]
fn effect_completed_cancelled_with_pause_applies_deferred_pause() {
    let mut view = running_recovery_view(EffectKind::Reconnect, RecoveryStrength::Reconnect);
    if let Some(effect) = view.effect.as_mut() {
        effect.cancel_reason = Some(CancelReason::Pause);
    }
    let d = tr(
        &view,
        Input::EffectCompleted {
            action_id: 1,
            kind: EffectKind::Reconnect,
            policy_revision: 1,
            outcome: EffectOutcome::Cancelled,
        },
    );
    assert!(has(&d, MachineInput::Execution(ExecutionInput::Cancelled)));
    assert!(has(
        &d,
        MachineInput::RetryGate {
            domain: RetryDomain::Recovery,
            input: RetryGateInput::ExplicitPause
        }
    ));
}

#[test]
fn effect_completed_aborted_panic_parks_as_invariant() {
    let view = running_recovery_view(EffectKind::Reconnect, RecoveryStrength::Reconnect);
    let d = tr(
        &view,
        Input::EffectCompleted {
            action_id: 1,
            kind: EffectKind::Reconnect,
            policy_revision: 1,
            outcome: EffectOutcome::Aborted {
                cause: AbortCause::PanicOrContractViolation,
            },
        },
    );
    assert!(d.parks.contains(&ParkDirective {
        domain: RetryDomain::Recovery,
        release_mask: ReleaseMask::single(ParkCause::InvariantViolation)
    }));
}

// ---------------------------------------------------------------------------
// EffectCompleted: stale failure of superseded / re-dispatched work
//
// A completion whose obligation was superseded or re-dispatched (its record is
// gone, or its work revision is newer than the effect's captured revision) must
// release the execution slot and do nothing else. Without the still-current
// guard, a stale probe failure could back off, escalate, or resurrect newer
// work.
// ---------------------------------------------------------------------------

/// A probe running under captured revision 1 whose intent was already
/// re-dispatched to a restore obligation at the newer revision 2.
fn stale_probe_over_new_restore() -> View {
    let mut view = View::initial();
    view.recovery_intent = RecoveryIntentState::RestorePending;
    view.recovery_record = Some(PendingRecord::recovery(2, RecoveryStrength::Restore));
    view.execution = ExecutionState::Probing;
    view.effect = Some(recovery_effect(1, EffectKind::Probe, 1));
    view.policy_revision = 2;
    view
}

fn stale_probe_completed(view: &View, diagnosis: EffectDiagnosis) -> Decision {
    tr(
        view,
        Input::EffectCompleted {
            action_id: 1,
            kind: EffectKind::Probe,
            policy_revision: 1,
            outcome: EffectOutcome::Failed { diagnosis },
        },
    )
}

#[test]
fn stale_probe_retry_does_not_back_off_re_dispatched_restore() {
    // Scenario 1: an availability failure would normally Retry, but here it must
    // not push the re-dispatched restore record into BackingOff or arm a backoff.
    let view = stale_probe_over_new_restore();
    let d = stale_probe_completed(
        &view,
        EffectDiagnosis::PathUnreachable {
            stage: "connect".into(),
        },
    );
    assert!(has(&d, MachineInput::Execution(ExecutionInput::Failed)));
    assert!(!has(
        &d,
        MachineInput::RetryGate {
            domain: RetryDomain::Recovery,
            input: RetryGateInput::RetryableFailure
        }
    ));
    assert!(!d.timers.iter().any(|t| matches!(
        t,
        TimerDirective::Arm {
            id: TimerId::FailureBackoff(_),
            ..
        }
    )));
}

#[test]
fn stale_probe_escalation_does_not_rewrite_re_dispatched_restore() {
    // Scenario 2: a conclusive failure would normally Escalate to Reconnect, but
    // here it must not overwrite the re-dispatched restore record's strength.
    let view = stale_probe_over_new_restore();
    let d = stale_probe_completed(&view, EffectDiagnosis::GenerationDead { generation: 5 });
    assert!(has(&d, MachineInput::Execution(ExecutionInput::Failed)));
    assert!(!has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestReconnect)
    ));
}

#[test]
fn stale_probe_failure_does_not_resurrect_intent_after_cleanup() {
    // Scenario 3: cleanup superseded intent to Idle (no recovery record). A stale
    // probe failure must not resurrect a reconnect obligation out of thin air,
    // and must leave the cleanup obligation untouched.
    let mut view = View::initial();
    view.recovery_intent = RecoveryIntentState::Idle;
    view.recovery_record = None;
    view.cleanup_work = CleanupWorkState::CleanupPending;
    view.cleanup_record = Some(PendingRecord::teardown(2));
    view.execution = ExecutionState::Probing;
    view.effect = Some(recovery_effect(1, EffectKind::Probe, 1));
    view.policy_revision = 2;
    let d = stale_probe_completed(&view, EffectDiagnosis::GenerationDead { generation: 5 });
    assert!(has(&d, MachineInput::Execution(ExecutionInput::Failed)));
    assert!(!has(
        &d,
        MachineInput::RecoveryIntent(RecoveryIntentInput::RequestReconnect)
    ));
    assert!(!has(
        &d,
        MachineInput::CleanupWork(CleanupWorkInput::CompleteCleanup)
    ));
}

#[test]
fn stale_probe_abort_does_not_park_re_dispatched_restore() {
    // The same still-current guard covers Aborted: a panic-classified abort of
    // the superseded probe must not park the re-dispatched restore record.
    let view = stale_probe_over_new_restore();
    let d = tr(
        &view,
        Input::EffectCompleted {
            action_id: 1,
            kind: EffectKind::Probe,
            policy_revision: 1,
            outcome: EffectOutcome::Aborted {
                cause: AbortCause::PanicOrContractViolation,
            },
        },
    );
    assert!(has(&d, MachineInput::Execution(ExecutionInput::Cancelled)));
    assert!(d.parks.is_empty());
    assert!(!has(
        &d,
        MachineInput::RetryGate {
            domain: RetryDomain::Recovery,
            input: RetryGateInput::TerminalFailure
        }
    ));
}

// ---------------------------------------------------------------------------
// Composite action decision table
// ---------------------------------------------------------------------------

#[test]
fn composite_cleanup_shadows_all_lower_domains() {
    let mut view = View::initial();
    view.cleanup_work = CleanupWorkState::CleanupPending;
    // Not ready -> None (shadow without action).
    let mut backing = PendingRecord::teardown(1);
    backing.gate = RetryGateState::BackingOff;
    view.cleanup_record = Some(backing);
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Reconnect));
    assert_eq!(composite_action(&view), None);
    // Ready -> Cleanup.
    view.cleanup_record = Some(PendingRecord::teardown(1));
    assert_eq!(composite_action(&view), Some(Action::Cleanup));
}

#[test]
fn composite_offline_disconnect_requires_committed_offline_path() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::Offline;
    view.offline_work = OfflineWorkState::DisconnectPending;
    view.offline_record = Some(PendingRecord::teardown(1));
    assert_eq!(
        composite_action(&view),
        Some(Action::ConfirmedOfflineDisconnect)
    );
}

#[test]
fn composite_recovery_intent_selected_by_strength_when_eligible() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::Online;
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Reconnect));
    assert_eq!(composite_action(&view), Some(Action::Reconnect));

    view.recovery_intent = RecoveryIntentState::RestorePending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Restore));
    assert_eq!(composite_action(&view), Some(Action::Restore));

    view.recovery_intent = RecoveryIntentState::ProbePending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Probe));
    assert_eq!(composite_action(&view), Some(Action::Probe));
}

#[test]
fn composite_gated_unknown_phase_grants_no_recovery() {
    let mut view = View::initial();
    view.profile = LifecycleProfile::Gated;
    view.app_phase = AppPhaseState::Unknown;
    view.network_path = NetworkPathState::Online;
    view.recovery_intent = RecoveryIntentState::ProbePending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Probe));
    assert_eq!(composite_action(&view), None);
}

#[test]
fn composite_logged_out_grants_no_recovery() {
    let mut view = View::initial();
    view.recovery_mode = RecoveryModeState::LoggedOut;
    view.network_path = NetworkPathState::Online;
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Reconnect));
    assert_eq!(composite_action(&view), None);
}

#[test]
fn composite_offline_candidate_defers_recovery() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::OfflineCandidate;
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    view.recovery_record = Some(PendingRecord::recovery(1, RecoveryStrength::Reconnect));
    assert_eq!(composite_action(&view), None);
}

#[test]
fn composite_backing_off_recovery_is_not_ready() {
    let mut view = View::initial();
    view.network_path = NetworkPathState::Online;
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    let mut rec = PendingRecord::recovery(1, RecoveryStrength::Reconnect);
    rec.gate = RetryGateState::BackingOff;
    view.recovery_record = Some(rec);
    assert_eq!(composite_action(&view), None);
}

// ---------------------------------------------------------------------------
// Derived send projection
// ---------------------------------------------------------------------------

#[test]
fn send_projection_strictest_match_wins() {
    // Blocked: not Active.
    let mut view = View::initial();
    view.recovery_mode = RecoveryModeState::LoggedOut;
    assert_eq!(derive_send_policy(&view), SendProjection::Blocked);

    // Blocked: teardown pending even while Active and Online.
    let mut view = View::initial();
    view.network_path = NetworkPathState::Online;
    view.cleanup_work = CleanupWorkState::CleanupPending;
    assert_eq!(derive_send_policy(&view), SendProjection::Blocked);

    // Blocked: path Offline.
    let mut view = View::initial();
    view.network_path = NetworkPathState::Offline;
    assert_eq!(derive_send_policy(&view), SendProjection::Blocked);

    // ExistingOnly: Active, OfflineCandidate, no teardown.
    let mut view = View::initial();
    view.network_path = NetworkPathState::OfflineCandidate;
    assert_eq!(derive_send_policy(&view), SendProjection::ExistingOnly);

    // Normal: Active, Online, no teardown.
    let mut view = View::initial();
    view.network_path = NetworkPathState::Online;
    assert_eq!(derive_send_policy(&view), SendProjection::Normal);
}

// ---------------------------------------------------------------------------
// Determinism and ordering
// ---------------------------------------------------------------------------

#[test]
fn translation_is_byte_stable_for_identical_arguments() {
    let view = running_recovery_view(EffectKind::Reconnect, RecoveryStrength::Reconnect);
    let input = Input::EffectCompleted {
        action_id: 1,
        kind: EffectKind::Reconnect,
        policy_revision: 1,
        outcome: EffectOutcome::Failed {
            diagnosis: EffectDiagnosis::Timeout {
                stage: "handshake".into(),
            },
        },
    };
    let mut a = FixedEntropy::new(vec![0.3, 0.7]);
    let mut b = FixedEntropy::new(vec![0.3, 0.7]);
    let da = translate(&view, &input, Duration::from_secs(1), &config(), &mut a);
    let db = translate(&view, &input, Duration::from_secs(1), &config(), &mut b);
    assert_eq!(da, db);
}

#[test]
fn recovery_strength_is_totally_ordered() {
    assert!(RecoveryStrength::Probe < RecoveryStrength::Restore);
    assert!(RecoveryStrength::Restore < RecoveryStrength::Reconnect);
}
