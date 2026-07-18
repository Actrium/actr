//! Responsive connection-recovery supervisor (RFC-0400).
//!
//! The supervisor is the sole writer of lifecycle recovery policy. It holds the
//! discrete state of the eight normative YASM machines plus the extended context
//! the RFC calls the policy `View`, and it drives every transition through the
//! pure [`translate`](super::recovery_policy::translate) reducer: an accepted
//! input is turned into a [`Decision`], and the shell applies that decision to
//! the view through YASM-checked transitions. No branch here re-decides policy
//! with a hand-written `if` chain, and no transition uses `.expect()`: an
//! impossible transition is a defensive `ERROR` log, never a panic.
//!
//! Asynchronous execution, timers, and effect completion delivery live in the
//! surrounding actor shell ([`super::network_event`]); this module stays
//! synchronous and clock-free (it is handed `now`), so it is exhaustively
//! testable without a runtime.

use std::time::Duration;

use yasm::StateMachine;

use super::network_event::{
    AppLifecycleState, CleanupReason as WireCleanupReason, LONG_BACKGROUND_RECONNECT_THRESHOLD_MS,
    NetworkEvent, NetworkRecoveryAction, NetworkSnapshot, ReconnectReason,
};
use super::recovery_policy::PolicyInstant;
use super::recovery_policy::classification::{DefaultEntropy, EntropySource};
use super::recovery_policy::diagnosis::EffectKind;
use super::recovery_policy::translate as tp;

// ---------------------------------------------------------------------------
// The eight normative YASM machines
//
// Four are defined here; the other four live in `recovery_execution` and
// `recovery_policy::machines`. They are `pub(in crate::lifecycle)` so the
// canonical documentation generator can reference them.
// ---------------------------------------------------------------------------

pub(in crate::lifecycle) mod app_phase {
    use yasm::define_state_machine;

    define_state_machine! {
        name: AppPhaseMachine,
        states: {
            Unknown,
            Foreground,
            Background
        },
        inputs: {
            EnterForeground,
            EnterBackground
        },
        initial: Unknown,
        transitions: {
            Unknown + EnterForeground => Foreground,
            Unknown + EnterBackground => Background,

            Foreground + EnterForeground => Foreground,
            Foreground + EnterBackground => Background,

            Background + EnterForeground => Foreground,
            Background + EnterBackground => Background
        }
    }
}

pub(in crate::lifecycle) mod path {
    use yasm::define_state_machine;

    // RFC-0400 network-path machine: the offline commit is a semantic
    // `CommitOffline` input, not a timer-named `GraceExpired`.
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
            CommitOffline
        },
        initial: Unknown,
        transitions: {
            Unknown + ObserveUnknown => Unknown,
            Unknown + ObserveOnline => Online,
            Unknown + ObserveOffline => OfflineCandidate,
            Unknown + CommitOffline => Unknown,

            Online + ObserveUnknown => Unknown,
            Online + ObserveOnline => Online,
            Online + ObserveOffline => OfflineCandidate,
            Online + CommitOffline => Online,

            OfflineCandidate + ObserveUnknown => Unknown,
            OfflineCandidate + ObserveOnline => Online,
            OfflineCandidate + ObserveOffline => OfflineCandidate,
            OfflineCandidate + CommitOffline => Offline,

            Offline + ObserveUnknown => Unknown,
            Offline + ObserveOnline => Online,
            Offline + ObserveOffline => Offline,
            Offline + CommitOffline => Offline
        }
    }
}

pub(in crate::lifecycle) mod recovery {
    use yasm::define_state_machine;

    // RFC-0400 recovery-intent machine: cleanup lives in its own `CleanupWork`
    // domain, so intent has four states. `SupersedeRecovery` returns any pending
    // intent to `Idle` for revision-ordered cleanup or session supersession.
    define_state_machine! {
        name: RecoveryIntentMachine,
        states: {
            Idle,
            ProbePending,
            RestorePending,
            ReconnectPending
        },
        inputs: {
            RequestProbe,
            RequestRestore,
            RequestReconnect,
            CompleteProbe,
            CompleteRestore,
            CompleteReconnect,
            SupersedeRecovery
        },
        initial: Idle,
        transitions: {
            Idle + RequestProbe => ProbePending,
            Idle + RequestRestore => RestorePending,
            Idle + RequestReconnect => ReconnectPending,

            ProbePending + RequestProbe => ProbePending,
            ProbePending + RequestRestore => RestorePending,
            ProbePending + RequestReconnect => ReconnectPending,
            ProbePending + CompleteProbe => Idle,
            ProbePending + SupersedeRecovery => Idle,

            RestorePending + RequestProbe => RestorePending,
            RestorePending + RequestRestore => RestorePending,
            RestorePending + RequestReconnect => ReconnectPending,
            RestorePending + CompleteRestore => Idle,
            RestorePending + SupersedeRecovery => Idle,

            ReconnectPending + RequestProbe => ReconnectPending,
            ReconnectPending + RequestRestore => ReconnectPending,
            ReconnectPending + RequestReconnect => ReconnectPending,
            ReconnectPending + CompleteReconnect => Idle,
            ReconnectPending + SupersedeRecovery => Idle
        }
    }
}

pub(in crate::lifecycle) mod offline_work {
    use yasm::define_state_machine;

    // RFC-0400 offline-work machine carries no defensive `Idle` self-loops: the
    // completion-dispatch guard keeps stale completions away from the machine.
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

            DisconnectPending + RequestDisconnect => DisconnectPending,
            DisconnectPending + CompleteDisconnect => Idle,
            DisconnectPending + SupersedeDisconnect => Idle
        }
    }
}

// ---------------------------------------------------------------------------
// Stable fact model (kept for the compatibility batch selector and callers)
// ---------------------------------------------------------------------------

/// Stable fact model accepted by the connection supervisor.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConnectionFact {
    NetworkSnapshotChanged(NetworkSnapshot),
    AppEnteredBackground,
    AppEnteredForeground { background_duration_ms: u64 },
    CleanupRequested(WireCleanupReason),
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

// ---------------------------------------------------------------------------
// Shell-facing decision outputs
// ---------------------------------------------------------------------------

/// A concrete arm/cancel instruction for the shell's timer facade.
///
/// Each op carries the fully-formed expiry input the shell delivers, so the
/// supervisor stays the single source of the candidate, retry, and deadline
/// identities that the expiry must echo back.
#[derive(Debug, Clone)]
pub(crate) enum TimerOp {
    Arm {
        key: tp::TimerId,
        at: PolicyInstant,
        fire: tp::Input,
    },
    Cancel {
        key: tp::TimerId,
    },
}

/// What the shell must do after one accepted input.
#[derive(Debug, Clone)]
pub(crate) struct AcceptOutcome {
    /// Whether the input advanced the policy revision.
    pub advanced: bool,
    /// Timer arm/cancel instructions in application order.
    pub timer_ops: Vec<TimerOp>,
    /// Whether the running effect must be cancelled.
    pub cancel_effect: bool,
    /// Structured status-stream records produced by translation.
    pub status: Vec<tp::StatusRecord>,
}

/// A newly started lifecycle effect the shell must spawn.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StartedEffect {
    pub action_id: u64,
    pub kind: EffectKind,
    pub action: tp::Action,
    pub captured_revision: u64,
    /// The remaining overall-teardown budget for a teardown effect, computed
    /// from its armed obligation deadline. `None` for non-teardown effects.
    pub teardown_budget: Option<Duration>,
}

// ---------------------------------------------------------------------------
// The responsive supervisor engine
// ---------------------------------------------------------------------------

/// The persistent, layered recovery supervisor.
pub struct ConnectionSupervisor {
    view: tp::View,
    config: tp::PolicyConfig,
    entropy: Box<dyn EntropySource + Send>,
    next_action_id: u64,
    next_candidate_id: u64,
    next_retry_id: u64,
    next_deadline_id: u64,
    pending_ops: Vec<TimerOp>,
}

impl std::fmt::Debug for ConnectionSupervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionSupervisor")
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

impl Default for ConnectionSupervisor {
    fn default() -> Self {
        Self::new(tp::LifecycleProfile::Ungated)
    }
}

impl ConnectionSupervisor {
    /// Construct a supervisor with the given lifecycle profile. The Rust core
    /// and headless deployments use `Ungated`; mobile bindings use `Gated`.
    pub(crate) fn new(profile: tp::LifecycleProfile) -> Self {
        Self::new_with_entropy(
            profile,
            Box::new(DefaultEntropy::seeded(next_entropy_seed())),
        )
    }

    /// Construct with an injected entropy source (deterministic tests).
    pub(crate) fn new_with_entropy(
        profile: tp::LifecycleProfile,
        entropy: Box<dyn EntropySource + Send>,
    ) -> Self {
        let mut view = tp::View::initial();
        view.profile = profile;
        Self {
            view,
            config: tp::PolicyConfig::defaults(),
            entropy,
            next_action_id: 1,
            next_candidate_id: 1,
            next_retry_id: 1,
            next_deadline_id: 1,
            pending_ops: Vec::new(),
        }
    }

    /// Read-only access to the policy view.
    pub(crate) fn view(&self) -> &tp::View {
        &self.view
    }

    /// The outbound-send admission projection of the current policy.
    pub(crate) fn send_policy(&self) -> tp::SendProjection {
        tp::derive_send_policy(&self.view)
    }

    /// The action the composite decision table currently selects, if any.
    pub(crate) fn composite_action(&self) -> Option<tp::Action> {
        tp::composite_action(&self.view)
    }

    /// The bootstrap-deadline arm op for a `Gated` supervisor, if any. The shell
    /// arms it once at start; the first authoritative phase cancels it.
    pub(crate) fn bootstrap_arm(&self, now: PolicyInstant) -> Option<TimerOp> {
        if self.view.profile == tp::LifecycleProfile::Gated {
            Some(TimerOp::Arm {
                key: tp::TimerId::BootstrapPhase,
                at: now + self.config.bootstrap_phase_deadline,
                fire: tp::Input::BootstrapPhaseDeadlineExpired,
            })
        } else {
            None
        }
    }

    /// Accept one input: translate it, apply the decision through checked
    /// transitions, maintain extended context, and return the shell's work.
    pub(crate) fn accept(&mut self, input: tp::Input, now: PolicyInstant) -> AcceptOutcome {
        let decision = tp::translate(&self.view, &input, now, &self.config, &mut *self.entropy);
        self.pending_ops.clear();

        let advanced = decision.revision == tp::RevisionDirective::Advances;
        if advanced {
            self.view.policy_revision = self.view.policy_revision.saturating_add(1);
        }
        let rev = self.view.policy_revision;

        for mi in &decision.machine_inputs {
            self.apply_machine_input(*mi, advanced, rev, now);
        }
        for gt in &decision.gate_triggers {
            self.apply_gate_trigger(*gt, rev);
        }
        for pk in &decision.parks {
            self.apply_park(pk);
        }
        self.apply_input_extended(&input, now, advanced);
        for td in &decision.timers {
            self.push_timer_op(td);
        }

        let cancel_effect = !decision.cancels.is_empty();
        if let Some(reason) = decision.cancels.first()
            && let Some(effect) = self.view.effect.as_mut()
            && effect.cancel_reason.is_none()
        {
            effect.cancel_reason = Some(*reason);
        }

        AcceptOutcome {
            advanced,
            timer_ops: std::mem::take(&mut self.pending_ops),
            cancel_effect,
            status: decision.status,
        }
    }

    /// If the execution slot is idle, select and begin the highest-priority
    /// action, recording its [`tp::EffectContext`]. Returns the started effect
    /// so the shell can spawn its task.
    pub(crate) fn maybe_start_effect(&mut self, now: PolicyInstant) -> Option<StartedEffect> {
        if self.view.execution != tp::ExecutionState::Idle {
            return None;
        }
        let action = tp::composite_action(&self.view)?;
        let kind = effect_kind_of(action);
        let action_id = self.next_action_id;
        self.next_action_id += 1;
        self.apply_execution_begin(action);
        if kind.is_teardown()
            && let Some(g) = self.view.live_signaling_generation
        {
            self.view.teardown_scope_generations.insert(g);
        }
        // A teardown effect inherits the remaining budget of its overall
        // teardown deadline (armed once when the obligation was created). The
        // in-effect budget is the "in-effect expiry" the translation table
        // defers to while the teardown effect runs.
        let teardown_budget = if kind.is_teardown() {
            let deadline = match kind {
                EffectKind::Cleanup => self.view.cleanup_teardown.map(|t| t.deadline),
                EffectKind::ConfirmedOfflineDisconnect => {
                    self.view.offline_teardown.map(|t| t.deadline)
                }
                _ => None,
            };
            Some(
                deadline
                    .map(|d| d.saturating_sub(now))
                    .unwrap_or(self.config.cleanup_teardown_deadline),
            )
        } else {
            None
        };
        let captured_revision = self.view.policy_revision;
        self.view.effect = Some(tp::EffectContext {
            action_id,
            kind,
            captured_revision,
            cancel_reason: None,
        });
        Some(StartedEffect {
            action_id,
            kind,
            action,
            captured_revision,
            teardown_budget,
        })
    }

    // -- input-specific extended state -------------------------------------

    fn apply_input_extended(&mut self, input: &tp::Input, now: PolicyInstant, advanced: bool) {
        match input {
            tp::Input::NetworkSnapshot {
                source_epoch,
                sequence,
                semantic_path,
                route_fingerprint,
            } => {
                // Record the last accepted snapshot on any accepted (newer)
                // (epoch, sequence), even a structural duplicate, so the epoch
                // ordering keeps advancing.
                if snapshot_accepted(&self.view.last_snapshot, *source_epoch, *sequence) {
                    self.view.last_snapshot = Some(tp::AcceptedSnapshot {
                        source_epoch: *source_epoch,
                        sequence: *sequence,
                        semantic_path: *semantic_path,
                        route_fingerprint: *route_fingerprint,
                    });
                }
            }
            tp::Input::SessionActivated { session_generation } if advanced => {
                self.view.committed_session_generation = Some(*session_generation);
            }
            tp::Input::SignalingGenerationCommitted { generation, .. } if advanced => {
                self.view.live_signaling_generation = Some(*generation);
            }
            tp::Input::SignalingGenerationLost { generation, .. }
                if advanced && self.view.live_signaling_generation == Some(*generation) =>
            {
                self.view.live_signaling_generation = None;
            }
            _ => {}
        }
        // `background_entered_at` is stamped by the shell when the phase machine
        // actually enters `Background`; that is handled in `apply_app_phase`.
        let _ = now;
    }

    // -- machine-input application -----------------------------------------

    fn apply_machine_input(
        &mut self,
        mi: tp::MachineInput,
        advanced: bool,
        rev: u64,
        now: PolicyInstant,
    ) {
        match mi {
            tp::MachineInput::RecoveryMode(i) => self.apply_recovery_mode(i),
            tp::MachineInput::AppPhase(i) => self.apply_app_phase(i, now),
            tp::MachineInput::NetworkPath(i) => self.apply_network_path(i),
            tp::MachineInput::RecoveryIntent(i) => self.apply_recovery_intent(i, advanced, rev),
            tp::MachineInput::CleanupWork(i) => self.apply_cleanup_work(i, rev),
            tp::MachineInput::OfflineWork(i) => self.apply_offline_work(i),
            tp::MachineInput::RetryGate { domain, input } => {
                self.apply_retry_gate(domain, input, rev)
            }
            tp::MachineInput::Execution(i) => self.apply_execution(i),
        }
    }

    fn apply_recovery_mode(&mut self, i: tp::RecoveryModeInput) {
        use super::recovery_policy::machines::recovery_mode as m;
        let cur = match self.view.recovery_mode {
            tp::RecoveryModeState::Active => m::State::Active,
            tp::RecoveryModeState::LoggedOut => m::State::LoggedOut,
            tp::RecoveryModeState::Terminating => m::State::Terminating,
        };
        let inp = match i {
            tp::RecoveryModeInput::SessionActivated => m::Input::SessionActivated,
            tp::RecoveryModeInput::UserLoggedOut => m::Input::UserLoggedOut,
            tp::RecoveryModeInput::AppTerminating => m::Input::AppTerminating,
        };
        match <m::RecoveryModeMachine as StateMachine>::next_state(&cur, &inp) {
            Some(m::State::Active) => self.view.recovery_mode = tp::RecoveryModeState::Active,
            Some(m::State::LoggedOut) => self.view.recovery_mode = tp::RecoveryModeState::LoggedOut,
            Some(m::State::Terminating) => {
                self.view.recovery_mode = tp::RecoveryModeState::Terminating
            }
            None => {
                tracing::error!(input = ?i, "connection_supervisor.recovery_mode.invalid_transition")
            }
        }
    }

    fn apply_app_phase(&mut self, i: tp::AppPhaseInput, now: PolicyInstant) {
        let old = self.view.app_phase;
        let cur = match old {
            tp::AppPhaseState::Unknown => app_phase::State::Unknown,
            tp::AppPhaseState::Foreground => app_phase::State::Foreground,
            tp::AppPhaseState::Background => app_phase::State::Background,
        };
        let inp = match i {
            tp::AppPhaseInput::EnterForeground => app_phase::Input::EnterForeground,
            tp::AppPhaseInput::EnterBackground => app_phase::Input::EnterBackground,
        };
        let new = match <app_phase::AppPhaseMachine as StateMachine>::next_state(&cur, &inp) {
            Some(app_phase::State::Unknown) => tp::AppPhaseState::Unknown,
            Some(app_phase::State::Foreground) => tp::AppPhaseState::Foreground,
            Some(app_phase::State::Background) => tp::AppPhaseState::Background,
            None => {
                tracing::error!(input = ?i, "connection_supervisor.app_phase.invalid_transition");
                return;
            }
        };
        self.view.app_phase = new;
        if new == tp::AppPhaseState::Background && old != tp::AppPhaseState::Background {
            self.view.background_entered_at = Some(now);
        }
        // The first authoritative phase cancels the gated bootstrap deadline.
        if old == tp::AppPhaseState::Unknown && new != tp::AppPhaseState::Unknown {
            self.pending_ops.push(TimerOp::Cancel {
                key: tp::TimerId::BootstrapPhase,
            });
        }
    }

    fn apply_network_path(&mut self, i: tp::NetworkPathInput) {
        let cur = match self.view.network_path {
            tp::NetworkPathState::Unknown => path::State::Unknown,
            tp::NetworkPathState::Online => path::State::Online,
            tp::NetworkPathState::OfflineCandidate => path::State::OfflineCandidate,
            tp::NetworkPathState::Offline => path::State::Offline,
        };
        let inp = match i {
            tp::NetworkPathInput::ObserveOnline => path::Input::ObserveOnline,
            tp::NetworkPathInput::ObserveOffline => path::Input::ObserveOffline,
            tp::NetworkPathInput::ObserveUnknown => path::Input::ObserveUnknown,
            tp::NetworkPathInput::CommitOffline => path::Input::CommitOffline,
        };
        let new = match <path::NetworkPathMachine as StateMachine>::next_state(&cur, &inp) {
            Some(path::State::Unknown) => tp::NetworkPathState::Unknown,
            Some(path::State::Online) => tp::NetworkPathState::Online,
            Some(path::State::OfflineCandidate) => tp::NetworkPathState::OfflineCandidate,
            Some(path::State::Offline) => tp::NetworkPathState::Offline,
            None => {
                tracing::error!(input = ?i, "connection_supervisor.network_path.invalid_transition");
                return;
            }
        };
        self.view.network_path = new;
        // The offline candidate identity is only meaningful while the path is a
        // candidate; leaving that state drops it (and its timer).
        if new != tp::NetworkPathState::OfflineCandidate && self.view.offline_candidate.is_some() {
            self.view.offline_candidate = None;
        }
    }

    fn apply_recovery_intent(&mut self, i: tp::RecoveryIntentInput, advanced: bool, rev: u64) {
        let new = self.recovery_intent_next(i);
        let Some(new) = new else { return };
        self.view.recovery_intent = new;
        match i {
            tp::RecoveryIntentInput::RequestProbe
            | tp::RecoveryIntentInput::RequestRestore
            | tp::RecoveryIntentInput::RequestReconnect => {
                let strength = match new {
                    tp::RecoveryIntentState::ProbePending => tp::RecoveryStrength::Probe,
                    tp::RecoveryIntentState::RestorePending => tp::RecoveryStrength::Restore,
                    tp::RecoveryIntentState::ReconnectPending => tp::RecoveryStrength::Reconnect,
                    tp::RecoveryIntentState::Idle => return,
                };
                match self.view.recovery_record.as_mut() {
                    // Escalation (`EffectCompleted` never advances a revision):
                    // retain the causal work revision, arm a fresh `Ready` gate.
                    Some(rec) if !advanced => {
                        rec.gate = tp::RetryGateState::Ready;
                        rec.attempt = 0;
                        rec.retry_id = 0;
                        rec.release_mask = Default::default();
                        rec.strength = Some(strength);
                    }
                    // A fresh externally-requested or derived obligation.
                    _ => {
                        self.view.recovery_record =
                            Some(tp::PendingRecord::recovery(rev, strength));
                    }
                }
                self.pending_ops.push(TimerOp::Cancel {
                    key: tp::TimerId::FailureBackoff(tp::RetryDomain::Recovery),
                });
            }
            tp::RecoveryIntentInput::CompleteProbe
            | tp::RecoveryIntentInput::CompleteRestore
            | tp::RecoveryIntentInput::CompleteReconnect
            | tp::RecoveryIntentInput::SupersedeRecovery => {
                self.view.recovery_record = None;
                self.pending_ops.push(TimerOp::Cancel {
                    key: tp::TimerId::FailureBackoff(tp::RetryDomain::Recovery),
                });
            }
        }
    }

    fn recovery_intent_next(&self, i: tp::RecoveryIntentInput) -> Option<tp::RecoveryIntentState> {
        let cur = match self.view.recovery_intent {
            tp::RecoveryIntentState::Idle => recovery::State::Idle,
            tp::RecoveryIntentState::ProbePending => recovery::State::ProbePending,
            tp::RecoveryIntentState::RestorePending => recovery::State::RestorePending,
            tp::RecoveryIntentState::ReconnectPending => recovery::State::ReconnectPending,
        };
        let inp = match i {
            tp::RecoveryIntentInput::RequestProbe => recovery::Input::RequestProbe,
            tp::RecoveryIntentInput::RequestRestore => recovery::Input::RequestRestore,
            tp::RecoveryIntentInput::RequestReconnect => recovery::Input::RequestReconnect,
            tp::RecoveryIntentInput::CompleteProbe => recovery::Input::CompleteProbe,
            tp::RecoveryIntentInput::CompleteRestore => recovery::Input::CompleteRestore,
            tp::RecoveryIntentInput::CompleteReconnect => recovery::Input::CompleteReconnect,
            tp::RecoveryIntentInput::SupersedeRecovery => recovery::Input::SupersedeRecovery,
        };
        match <recovery::RecoveryIntentMachine as StateMachine>::next_state(&cur, &inp) {
            Some(recovery::State::Idle) => Some(tp::RecoveryIntentState::Idle),
            Some(recovery::State::ProbePending) => Some(tp::RecoveryIntentState::ProbePending),
            Some(recovery::State::RestorePending) => Some(tp::RecoveryIntentState::RestorePending),
            Some(recovery::State::ReconnectPending) => {
                Some(tp::RecoveryIntentState::ReconnectPending)
            }
            None => {
                tracing::error!(input = ?i, "connection_supervisor.recovery_intent.invalid_transition");
                None
            }
        }
    }

    fn apply_cleanup_work(&mut self, i: tp::CleanupWorkInput, rev: u64) {
        use super::recovery_policy::machines::cleanup_work as m;
        let cur = match self.view.cleanup_work {
            tp::CleanupWorkState::Idle => m::State::Idle,
            tp::CleanupWorkState::CleanupPending => m::State::CleanupPending,
        };
        let inp = match i {
            tp::CleanupWorkInput::RequestCleanup => m::Input::RequestCleanup,
            tp::CleanupWorkInput::CompleteCleanup => m::Input::CompleteCleanup,
        };
        match <m::CleanupWorkMachine as StateMachine>::next_state(&cur, &inp) {
            Some(m::State::Idle) => self.view.cleanup_work = tp::CleanupWorkState::Idle,
            Some(m::State::CleanupPending) => {
                self.view.cleanup_work = tp::CleanupWorkState::CleanupPending
            }
            None => {
                tracing::error!(input = ?i, "connection_supervisor.cleanup_work.invalid_transition");
                return;
            }
        }
        match i {
            tp::CleanupWorkInput::RequestCleanup => {
                self.view.cleanup_record = Some(tp::PendingRecord::teardown(rev));
            }
            tp::CleanupWorkInput::CompleteCleanup => {
                self.view.cleanup_record = None;
                if self.view.cleanup_teardown.take().is_some() {
                    self.pending_ops.push(TimerOp::Cancel {
                        key: tp::TimerId::TeardownOverall(tp::TeardownDomain::Cleanup),
                    });
                }
                self.pending_ops.push(TimerOp::Cancel {
                    key: tp::TimerId::FailureBackoff(tp::RetryDomain::Cleanup),
                });
            }
        }
    }

    fn apply_offline_work(&mut self, i: tp::OfflineWorkInput) {
        let cur = match self.view.offline_work {
            tp::OfflineWorkState::Idle => offline_work::State::Idle,
            tp::OfflineWorkState::DisconnectPending => offline_work::State::DisconnectPending,
        };
        let inp = match i {
            tp::OfflineWorkInput::RequestDisconnect => offline_work::Input::RequestDisconnect,
            tp::OfflineWorkInput::CompleteDisconnect => offline_work::Input::CompleteDisconnect,
            tp::OfflineWorkInput::SupersedeDisconnect => offline_work::Input::SupersedeDisconnect,
        };
        match <offline_work::OfflineWorkMachine as StateMachine>::next_state(&cur, &inp) {
            Some(offline_work::State::Idle) => self.view.offline_work = tp::OfflineWorkState::Idle,
            Some(offline_work::State::DisconnectPending) => {
                self.view.offline_work = tp::OfflineWorkState::DisconnectPending
            }
            None => {
                tracing::error!(input = ?i, "connection_supervisor.offline_work.invalid_transition");
                return;
            }
        }
        match i {
            tp::OfflineWorkInput::RequestDisconnect => {
                self.view.offline_record =
                    Some(tp::PendingRecord::teardown(self.view.policy_revision));
            }
            tp::OfflineWorkInput::CompleteDisconnect
            | tp::OfflineWorkInput::SupersedeDisconnect => {
                self.view.offline_record = None;
                if self.view.offline_teardown.take().is_some() {
                    self.pending_ops.push(TimerOp::Cancel {
                        key: tp::TimerId::TeardownOverall(tp::TeardownDomain::OfflineDisconnect),
                    });
                }
                self.pending_ops.push(TimerOp::Cancel {
                    key: tp::TimerId::FailureBackoff(tp::RetryDomain::Offline),
                });
            }
        }
    }

    fn apply_retry_gate(&mut self, domain: tp::RetryDomain, input: tp::RetryGateInput, rev: u64) {
        // Allocate a retry id up front so the record borrow stays short.
        let new_retry_id = if input == tp::RetryGateInput::RetryableFailure {
            let id = self.next_retry_id;
            self.next_retry_id += 1;
            id
        } else {
            0
        };
        let Some(rec) = self.record_mut(domain) else {
            tracing::error!(?domain, input = ?input, "connection_supervisor.retry_gate.missing_record");
            return;
        };
        let Some(next) = retry_gate_next(rec.gate, input) else {
            return;
        };
        rec.gate = next;
        match input {
            tp::RetryGateInput::RetryableFailure => {
                rec.attempt = rec.attempt.saturating_add(1);
                rec.retry_id = new_retry_id;
            }
            tp::RetryGateInput::RetryDeadlineExpired => {
                rec.work_revision = rev;
            }
            tp::RetryGateInput::NewMaterialTrigger => {
                rec.attempt = 0;
                rec.work_revision = rev;
                self.pending_ops.push(TimerOp::Cancel {
                    key: tp::TimerId::FailureBackoff(domain),
                });
            }
            tp::RetryGateInput::TerminalFailure => {
                // The release mask is recorded by the accompanying park directive.
            }
            tp::RetryGateInput::ExplicitPause => {
                self.pending_ops.push(TimerOp::Cancel {
                    key: tp::TimerId::FailureBackoff(domain),
                });
            }
        }
    }

    fn apply_gate_trigger(&mut self, gt: tp::GateTrigger, rev: u64) {
        match gt {
            tp::GateTrigger::Wake { domain } => {
                let Some(rec) = self.record_mut(domain) else {
                    return;
                };
                if rec.gate != tp::RetryGateState::BackingOff {
                    return;
                }
                if let Some(next) =
                    retry_gate_next(rec.gate, tp::RetryGateInput::NewMaterialTrigger)
                {
                    rec.gate = next;
                    rec.attempt = 0;
                    rec.work_revision = rev;
                    self.pending_ops.push(TimerOp::Cancel {
                        key: tp::TimerId::FailureBackoff(domain),
                    });
                }
            }
            tp::GateTrigger::ClearMask { domain, trigger } => {
                let Some(rec) = self.record_mut(domain) else {
                    return;
                };
                if rec.gate != tp::RetryGateState::Parked {
                    return;
                }
                rec.release_mask.clear_matching(trigger);
                if rec.release_mask.is_empty()
                    && let Some(next) =
                        retry_gate_next(rec.gate, tp::RetryGateInput::NewMaterialTrigger)
                {
                    rec.gate = next;
                    rec.attempt = 0;
                    rec.work_revision = rev;
                }
            }
        }
    }

    fn apply_park(&mut self, pk: &tp::ParkDirective) {
        if let Some(rec) = self.record_mut(pk.domain) {
            rec.release_mask.union_with(&pk.release_mask);
        }
    }

    fn apply_execution(&mut self, i: tp::ExecutionInput) {
        use super::recovery_execution::{Input as EI, RecoveryExecutionMachine as EM, State as ES};
        let cur = exec_to_yasm(self.view.execution);
        let inp = match i {
            tp::ExecutionInput::BeginProbe => EI::BeginProbe,
            tp::ExecutionInput::BeginRestore => EI::BeginRestore,
            tp::ExecutionInput::BeginReconnect => EI::BeginReconnect,
            tp::ExecutionInput::BeginOffline => EI::BeginOffline,
            tp::ExecutionInput::BeginCleanup => EI::BeginCleanup,
            tp::ExecutionInput::Succeeded => EI::Succeeded,
            tp::ExecutionInput::Failed => EI::Failed,
            tp::ExecutionInput::Cancelled => EI::Cancelled,
        };
        let Some(next) = <EM as StateMachine>::next_state(&cur, &inp) else {
            tracing::error!(input = ?i, "connection_supervisor.execution.invalid_transition");
            return;
        };
        // A completion (any input returning to Idle) releases the effect context;
        // a successful teardown additionally drops its scoped live generations.
        if matches!(
            i,
            tp::ExecutionInput::Succeeded
                | tp::ExecutionInput::Failed
                | tp::ExecutionInput::Cancelled
        ) {
            let was_teardown = self
                .view
                .effect
                .as_ref()
                .map(|e| e.kind.is_teardown())
                .unwrap_or(false);
            if i == tp::ExecutionInput::Succeeded && was_teardown {
                for g in std::mem::take(&mut self.view.teardown_scope_generations) {
                    if self.view.live_signaling_generation == Some(g) {
                        self.view.live_signaling_generation = None;
                    }
                }
            }
            self.view.effect = None;
        }
        self.view.execution = exec_from_yasm(next);
        let _ = ES::Idle;
    }

    fn apply_execution_begin(&mut self, action: tp::Action) {
        let input = match action {
            tp::Action::Probe => tp::ExecutionInput::BeginProbe,
            tp::Action::Restore => tp::ExecutionInput::BeginRestore,
            tp::Action::Reconnect => tp::ExecutionInput::BeginReconnect,
            tp::Action::ConfirmedOfflineDisconnect => tp::ExecutionInput::BeginOffline,
            tp::Action::Cleanup => tp::ExecutionInput::BeginCleanup,
        };
        self.apply_execution(input);
    }

    // -- timer directive lowering ------------------------------------------

    fn push_timer_op(&mut self, td: &tp::TimerDirective) {
        match td {
            tp::TimerDirective::Arm { id, deadline, .. } => match id {
                tp::TimerId::OfflineCandidate => {
                    let candidate_id = self.next_candidate_id;
                    self.next_candidate_id += 1;
                    self.view.offline_candidate = Some(tp::OfflineCandidate {
                        candidate_id,
                        deadline: *deadline,
                    });
                    self.pending_ops.push(TimerOp::Arm {
                        key: *id,
                        at: *deadline,
                        fire: tp::Input::OfflineGraceExpired { candidate_id },
                    });
                }
                tp::TimerId::ShutdownOverall => {
                    let deadline_id = self.next_deadline_id;
                    self.next_deadline_id += 1;
                    self.view.shutdown_deadline = Some(tp::ShutdownDeadline {
                        deadline_id,
                        deadline: *deadline,
                    });
                    self.pending_ops.push(TimerOp::Arm {
                        key: *id,
                        at: *deadline,
                        fire: tp::Input::ShutdownDeadlineExpired { deadline_id },
                    });
                }
                tp::TimerId::TeardownOverall(domain) => {
                    let deadline_id = self.next_deadline_id;
                    self.next_deadline_id += 1;
                    let record = tp::TeardownDeadline {
                        deadline_id,
                        deadline: *deadline,
                    };
                    match domain {
                        tp::TeardownDomain::Cleanup => self.view.cleanup_teardown = Some(record),
                        tp::TeardownDomain::OfflineDisconnect => {
                            self.view.offline_teardown = Some(record)
                        }
                    }
                    self.pending_ops.push(TimerOp::Arm {
                        key: *id,
                        at: *deadline,
                        fire: tp::Input::TeardownDeadlineExpired {
                            domain: *domain,
                            deadline_id,
                        },
                    });
                }
                tp::TimerId::FailureBackoff(domain) => {
                    let (work_revision, retry_id) = self
                        .view
                        .record(*domain)
                        .map(|r| (r.work_revision, r.retry_id))
                        .unwrap_or((0, 0));
                    self.pending_ops.push(TimerOp::Arm {
                        key: *id,
                        at: *deadline,
                        fire: tp::Input::RetryDeadlineExpired {
                            domain: *domain,
                            work_revision,
                            retry_id,
                        },
                    });
                }
                tp::TimerId::BootstrapPhase => {
                    self.pending_ops.push(TimerOp::Arm {
                        key: *id,
                        at: *deadline,
                        fire: tp::Input::BootstrapPhaseDeadlineExpired,
                    });
                }
            },
            tp::TimerDirective::Cancel { id } => {
                match id {
                    tp::TimerId::OfflineCandidate => self.view.offline_candidate = None,
                    tp::TimerId::ShutdownOverall => self.view.shutdown_deadline = None,
                    tp::TimerId::TeardownOverall(tp::TeardownDomain::Cleanup) => {
                        self.view.cleanup_teardown = None
                    }
                    tp::TimerId::TeardownOverall(tp::TeardownDomain::OfflineDisconnect) => {
                        self.view.offline_teardown = None
                    }
                    tp::TimerId::FailureBackoff(_) | tp::TimerId::BootstrapPhase => {}
                }
                self.pending_ops.push(TimerOp::Cancel { key: *id });
            }
        }
    }

    fn record_mut(&mut self, domain: tp::RetryDomain) -> Option<&mut tp::PendingRecord> {
        match domain {
            tp::RetryDomain::Recovery => self.view.recovery_record.as_mut(),
            tp::RetryDomain::Cleanup => self.view.cleanup_record.as_mut(),
            tp::RetryDomain::Offline => self.view.offline_record.as_mut(),
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

fn effect_kind_of(action: tp::Action) -> EffectKind {
    match action {
        tp::Action::Cleanup => EffectKind::Cleanup,
        tp::Action::ConfirmedOfflineDisconnect => EffectKind::ConfirmedOfflineDisconnect,
        tp::Action::Reconnect => EffectKind::Reconnect,
        tp::Action::Restore => EffectKind::Restore,
        tp::Action::Probe => EffectKind::Probe,
    }
}

/// Map a composite [`tp::Action`] onto the wire recovery-action vocabulary the
/// effect processor still executes.
pub(crate) fn network_action_of(action: tp::Action) -> NetworkRecoveryAction {
    match action {
        tp::Action::Cleanup => NetworkRecoveryAction::CleanupOnly,
        tp::Action::ConfirmedOfflineDisconnect => NetworkRecoveryAction::Offline,
        tp::Action::Reconnect => NetworkRecoveryAction::ForceReconnect,
        tp::Action::Restore => NetworkRecoveryAction::Restore,
        tp::Action::Probe => NetworkRecoveryAction::Probe,
    }
}

fn exec_to_yasm(s: tp::ExecutionState) -> super::recovery_execution::State {
    use super::recovery_execution::State as ES;
    match s {
        tp::ExecutionState::Idle => ES::Idle,
        tp::ExecutionState::Disconnecting => ES::Disconnecting,
        tp::ExecutionState::Probing => ES::Probing,
        tp::ExecutionState::Restoring => ES::Restoring,
        tp::ExecutionState::Reconnecting => ES::Reconnecting,
        tp::ExecutionState::Cleaning => ES::Cleaning,
    }
}

fn exec_from_yasm(s: super::recovery_execution::State) -> tp::ExecutionState {
    use super::recovery_execution::State as ES;
    match s {
        ES::Idle => tp::ExecutionState::Idle,
        ES::Disconnecting => tp::ExecutionState::Disconnecting,
        ES::Probing => tp::ExecutionState::Probing,
        ES::Restoring => tp::ExecutionState::Restoring,
        ES::Reconnecting => tp::ExecutionState::Reconnecting,
        ES::Cleaning => tp::ExecutionState::Cleaning,
    }
}

fn retry_gate_next(
    cur: tp::RetryGateState,
    input: tp::RetryGateInput,
) -> Option<tp::RetryGateState> {
    use super::recovery_policy::machines::retry_gate as g;
    let c = match cur {
        tp::RetryGateState::Ready => g::State::Ready,
        tp::RetryGateState::BackingOff => g::State::BackingOff,
        tp::RetryGateState::Parked => g::State::Parked,
    };
    let i = match input {
        tp::RetryGateInput::RetryableFailure => g::Input::RetryableFailure,
        tp::RetryGateInput::TerminalFailure => g::Input::TerminalFailure,
        tp::RetryGateInput::ExplicitPause => g::Input::ExplicitPause,
        tp::RetryGateInput::RetryDeadlineExpired => g::Input::RetryDeadlineExpired,
        tp::RetryGateInput::NewMaterialTrigger => g::Input::NewMaterialTrigger,
    };
    match <g::RetryGateMachine as StateMachine>::next_state(&c, &i) {
        Some(g::State::Ready) => Some(tp::RetryGateState::Ready),
        Some(g::State::BackingOff) => Some(tp::RetryGateState::BackingOff),
        Some(g::State::Parked) => Some(tp::RetryGateState::Parked),
        None => {
            tracing::error!(state = ?cur, input = ?input, "connection_supervisor.retry_gate.invalid_transition");
            None
        }
    }
}

/// Whether a `(source_epoch, sequence)` is accepted against the last snapshot,
/// by lexicographic `(epoch, sequence)` order.
fn snapshot_accepted(last: &Option<tp::AcceptedSnapshot>, epoch: u64, sequence: u64) -> bool {
    match last {
        None => true,
        Some(last) => match epoch.cmp(&last.source_epoch) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => sequence > last.sequence,
        },
    }
}

fn next_entropy_seed() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEED: AtomicU64 = AtomicU64::new(0xD1B5_4A32_D192_ED03);
    SEED.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Compatibility batch selector
//
// `select_network_recovery_action` and `process_network_event_batch` are legacy
// synchronous helpers with no timer owner. They preserve the pre-RFC selection
// behavior so existing batch callers keep their contract while the responsive
// reconciler drives the RFC engine.
// ---------------------------------------------------------------------------

pub(crate) fn legacy_select_action(events: &[NetworkEvent]) -> NetworkRecoveryAction {
    let mut sel = legacy::LegacySelector::default();
    for event in events {
        sel.submit(&ConnectionFact::from_network_event(event));
    }
    if sel.offline_grace_pending() {
        sel.expire_offline_grace();
    }
    sel.reconcile()
}

mod legacy {
    use super::{ConnectionFact, NetworkRecoveryAction};
    use crate::lifecycle::network_event::{
        LONG_BACKGROUND_RECONNECT_THRESHOLD_MS, NetworkAvailability,
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Phase {
        Unknown,
        Foreground,
        Background,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Path {
        Unknown,
        Online,
        OfflineCandidate,
        Offline,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Intent {
        Idle,
        Probe,
        Restore,
        Reconnect,
        Cleanup,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Work {
        Idle,
        DisconnectPending,
    }

    /// The pre-RFC layered selection logic ported to plain enums, preserving the
    /// exact batch outcome the compatibility helpers documented.
    pub(super) struct LegacySelector {
        phase: Phase,
        path: Path,
        intent: Intent,
        work: Work,
        latest_sequence: Option<u64>,
        latest_route: Option<(NetworkAvailability, u64)>,
    }

    impl Default for LegacySelector {
        fn default() -> Self {
            Self {
                phase: Phase::Unknown,
                path: Path::Unknown,
                intent: Intent::Idle,
                work: Work::Idle,
                latest_sequence: None,
                latest_route: None,
            }
        }
    }

    impl LegacySelector {
        pub(super) fn submit(&mut self, fact: &ConnectionFact) {
            match fact {
                ConnectionFact::CleanupRequested(_) => self.intent = Intent::Cleanup,
                ConnectionFact::ForceReconnectRequested(_) => self.raise(Intent::Reconnect),
                ConnectionFact::AppEnteredBackground => self.phase = Phase::Background,
                ConnectionFact::AppEnteredForeground {
                    background_duration_ms,
                } => {
                    self.phase = Phase::Foreground;
                    if *background_duration_ms >= LONG_BACKGROUND_RECONNECT_THRESHOLD_MS {
                        self.raise(Intent::Reconnect);
                    } else {
                        self.raise(Intent::Probe);
                    }
                }
                ConnectionFact::NetworkSnapshotChanged(snapshot) => {
                    if self
                        .latest_sequence
                        .is_some_and(|latest| snapshot.sequence <= latest)
                    {
                        return;
                    }
                    let route = (
                        snapshot.availability,
                        route_fingerprint(
                            snapshot.transport,
                            snapshot.is_expensive,
                            snapshot.is_constrained,
                        ),
                    );
                    let materially_changed = self.latest_route != Some(route);
                    self.latest_sequence = Some(snapshot.sequence);
                    self.latest_route = Some(route);
                    match snapshot.availability {
                        NetworkAvailability::Unknown => {
                            self.path = Path::Unknown;
                            self.work = Work::Idle;
                            if materially_changed {
                                self.raise(Intent::Probe);
                            }
                        }
                        NetworkAvailability::Available => {
                            self.path = Path::Online;
                            self.work = Work::Idle;
                            if materially_changed {
                                self.raise(Intent::Restore);
                            }
                        }
                        NetworkAvailability::Unavailable => {
                            self.path = Path::OfflineCandidate;
                        }
                    }
                }
            }
        }

        fn raise(&mut self, intent: Intent) {
            if rank(intent) > rank(self.intent) {
                self.intent = intent;
            }
        }

        pub(super) fn offline_grace_pending(&self) -> bool {
            self.path == Path::OfflineCandidate
        }

        pub(super) fn expire_offline_grace(&mut self) {
            if self.path == Path::OfflineCandidate {
                self.path = Path::Offline;
                self.work = Work::DisconnectPending;
            }
        }

        pub(super) fn reconcile(&self) -> NetworkRecoveryAction {
            if self.intent == Intent::Cleanup {
                return NetworkRecoveryAction::CleanupOnly;
            }
            if self.work == Work::DisconnectPending {
                return NetworkRecoveryAction::Offline;
            }
            if matches!(self.path, Path::OfflineCandidate | Path::Offline) {
                return NetworkRecoveryAction::Noop;
            }
            if self.phase == Phase::Background {
                return NetworkRecoveryAction::Noop;
            }
            match self.intent {
                Intent::Idle => NetworkRecoveryAction::Noop,
                Intent::Probe => NetworkRecoveryAction::Probe,
                Intent::Restore => NetworkRecoveryAction::Restore,
                Intent::Reconnect => NetworkRecoveryAction::ForceReconnect,
                Intent::Cleanup => NetworkRecoveryAction::CleanupOnly,
            }
        }
    }

    fn rank(intent: Intent) -> u8 {
        match intent {
            Intent::Idle => 0,
            Intent::Probe => 1,
            Intent::Restore => 2,
            Intent::Reconnect => 3,
            Intent::Cleanup => 4,
        }
    }

    fn route_fingerprint(
        transport: crate::lifecycle::network_event::NetworkTransportFlags,
        is_expensive: bool,
        is_constrained: bool,
    ) -> u64 {
        let mut fp = 0u64;
        fp |= transport.wifi as u64;
        fp |= (transport.cellular as u64) << 1;
        fp |= (transport.ethernet as u64) << 2;
        fp |= (transport.vpn as u64) << 3;
        fp |= (transport.other as u64) << 4;
        fp |= (is_expensive as u64) << 5;
        fp |= (is_constrained as u64) << 6;
        fp
    }
}

/// Convert a public [`NetworkEvent`] into an RFC supervisor [`tp::Input`].
///
/// `source_epoch` is stamped by the [`super::network_event::NetworkEventHandle`]
/// at construction; existing callers keep their sequence semantics unchanged.
pub(crate) fn event_to_input(event: &NetworkEvent, source_epoch: u64) -> tp::Input {
    match event {
        NetworkEvent::NetworkPathChanged { snapshot } => tp::Input::NetworkSnapshot {
            source_epoch,
            sequence: snapshot.sequence,
            semantic_path: match snapshot.availability {
                super::network_event::NetworkAvailability::Unknown => tp::SemanticPath::Unknown,
                super::network_event::NetworkAvailability::Available => tp::SemanticPath::Online,
                super::network_event::NetworkAvailability::Unavailable => tp::SemanticPath::Offline,
            },
            route_fingerprint: route_fingerprint(snapshot),
        },
        NetworkEvent::AppLifecycleChanged { state } => match state {
            AppLifecycleState::Background => tp::Input::AppEnteredBackground,
            AppLifecycleState::Foreground { .. } => tp::Input::AppEnteredForeground,
        },
        NetworkEvent::CleanupConnections { reason } => tp::Input::CleanupRequested {
            reason: match reason {
                WireCleanupReason::AppTerminating => tp::CleanupReason::AppTerminating,
                WireCleanupReason::UserLogout => tp::CleanupReason::UserLogout,
                WireCleanupReason::StaleConnectionSuspected => {
                    tp::CleanupReason::StaleConnectionSuspected
                }
                WireCleanupReason::ManualReset => tp::CleanupReason::ManualReset,
            },
        },
        NetworkEvent::ForceReconnect { reason } => tp::Input::RecoveryRequested {
            minimum: tp::RecoveryStrength::Reconnect,
            reason: match reason {
                ReconnectReason::NetworkPathChanged => {
                    tp::RecoveryRequestReason::NetworkPathChanged
                }
                ReconnectReason::LongBackground => tp::RecoveryRequestReason::LongBackground,
                ReconnectReason::ProbeFailed => tp::RecoveryRequestReason::ProbeFailed,
                ReconnectReason::ManualReconnect => tp::RecoveryRequestReason::ManualReconnect,
                ReconnectReason::StaleConnectionSuspected => {
                    tp::RecoveryRequestReason::StaleConnectionSuspected
                }
            },
        },
    }
}

fn route_fingerprint(snapshot: &NetworkSnapshot) -> u64 {
    let t = snapshot.transport;
    let mut fp = 0u64;
    fp |= t.wifi as u64;
    fp |= (t.cellular as u64) << 1;
    fp |= (t.ethernet as u64) << 2;
    fp |= (t.vpn as u64) << 3;
    fp |= (t.other as u64) << 4;
    fp |= (snapshot.is_expensive as u64) << 5;
    fp |= (snapshot.is_constrained as u64) << 6;
    fp
}

/// The long-background threshold is retained for the compatibility selector.
const _: Duration = Duration::from_millis(LONG_BACKGROUND_RECONNECT_THRESHOLD_MS);

#[cfg(test)]
#[path = "connection_supervisor_tests.rs"]
mod tests;
