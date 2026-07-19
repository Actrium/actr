//! The pure policy translation reducer (RFC-0400 "Policy translation").
//!
//! [`translate`] is the single place in the supervisor that turns one accepted
//! input into a [`Decision`]: an ordered list of machine inputs plus the
//! revision, trigger, cancellation, and timer consequences the actor shell
//! then applies. It performs no I/O, reads no ambient clock (it is handed
//! `now`), and draws no ambient randomness (it is handed an [`EntropySource`]);
//! identical arguments produce an identical, byte-comparable decision.
//!
//! The composite action decision ([`composite_action`]) and the derived send
//! projection ([`derive_send_policy`]) are the stage-2 pure functions that read
//! the same snapshot.

use std::collections::BTreeSet;
use std::time::Duration;

use super::classification::{
    BackoffParams, ClassifyContext, EntropySource, ReleaseMask, TriggerClass, Verdict, arm_backoff,
    classify,
};
use super::diagnosis::{AbortCause, EffectDiagnosis, EffectKind, EffectOutcome};
use super::{Generation, PolicyInstant, Revision};

// ---------------------------------------------------------------------------
// Discrete machine states (the reducer's snapshot alphabet)
// ---------------------------------------------------------------------------

/// `RecoveryMode` discrete state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RecoveryModeState {
    Active,
    LoggedOut,
    Terminating,
}

/// `AppPhase` discrete state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AppPhaseState {
    Unknown,
    Foreground,
    Background,
}

/// `NetworkPath` discrete state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NetworkPathState {
    Unknown,
    Online,
    OfflineCandidate,
    Offline,
}

/// `RecoveryIntent` discrete state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RecoveryIntentState {
    Idle,
    ProbePending,
    RestorePending,
    ReconnectPending,
}

/// `CleanupWork` discrete state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CleanupWorkState {
    Idle,
    CleanupPending,
}

/// `OfflineWork` discrete state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum OfflineWorkState {
    Idle,
    DisconnectPending,
}

/// `Execution` single-flight discrete state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ExecutionState {
    Idle,
    Disconnecting,
    Probing,
    Restoring,
    Reconnecting,
    Cleaning,
}

impl ExecutionState {
    fn is_recovery(self) -> bool {
        matches!(self, Self::Probing | Self::Restoring | Self::Reconnecting)
    }
}

/// `RetryGate` discrete state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RetryGateState {
    Ready,
    BackingOff,
    Parked,
}

// ---------------------------------------------------------------------------
// Machine inputs the decision emits
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryModeInput {
    SessionActivated,
    UserLoggedOut,
    AppTerminating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppPhaseInput {
    EnterForeground,
    EnterBackground,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NetworkPathInput {
    ObserveOnline,
    ObserveOffline,
    ObserveUnknown,
    CommitOffline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryIntentInput {
    RequestProbe,
    RequestRestore,
    RequestReconnect,
    CompleteProbe,
    CompleteRestore,
    CompleteReconnect,
    SupersedeRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CleanupWorkInput {
    RequestCleanup,
    CompleteCleanup,
}

// The variant names mirror the `OfflineWork` state-machine input vocabulary.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OfflineWorkInput {
    RequestDisconnect,
    CompleteDisconnect,
    SupersedeDisconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryGateInput {
    RetryableFailure,
    TerminalFailure,
    ExplicitPause,
    RetryDeadlineExpired,
    NewMaterialTrigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionInput {
    BeginProbe,
    BeginRestore,
    BeginReconnect,
    BeginOffline,
    BeginCleanup,
    Succeeded,
    Failed,
    Cancelled,
}

/// One machine input paired with its target machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MachineInput {
    RecoveryMode(RecoveryModeInput),
    AppPhase(AppPhaseInput),
    NetworkPath(NetworkPathInput),
    RecoveryIntent(RecoveryIntentInput),
    CleanupWork(CleanupWorkInput),
    OfflineWork(OfflineWorkInput),
    RetryGate {
        domain: RetryDomain,
        input: RetryGateInput,
    },
    Execution(ExecutionInput),
}

// ---------------------------------------------------------------------------
// Shared policy vocabulary
// ---------------------------------------------------------------------------

/// A retry-gated pending-work domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum RetryDomain {
    Recovery,
    Cleanup,
    Offline,
}

/// A bounded-teardown obligation domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum TeardownDomain {
    Cleanup,
    OfflineDisconnect,
}

/// The ordered strength of a recovery intent: `Probe < Restore < Reconnect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum RecoveryStrength {
    Probe,
    Restore,
    Reconnect,
}

impl RecoveryStrength {
    fn request_input(self) -> RecoveryIntentInput {
        match self {
            Self::Probe => RecoveryIntentInput::RequestProbe,
            Self::Restore => RecoveryIntentInput::RequestRestore,
            Self::Reconnect => RecoveryIntentInput::RequestReconnect,
        }
    }

    fn complete_input(self) -> RecoveryIntentInput {
        match self {
            Self::Probe => RecoveryIntentInput::CompleteProbe,
            Self::Restore => RecoveryIntentInput::CompleteRestore,
            Self::Reconnect => RecoveryIntentInput::CompleteReconnect,
        }
    }
}

/// The closed reason enumeration for a cleanup command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CleanupReason {
    UserLogout,
    AppTerminating,
    ManualReset,
    StaleConnectionSuspected,
}

/// A diagnostic reason attached to an explicit recovery command; it does not
/// affect translation, which keys only on `minimum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RecoveryRequestReason {
    ManualReconnect,
    NetworkPathChanged,
    LongBackground,
    ProbeFailed,
    StaleConnectionSuspected,
}

/// The scope of a recovery pause/resume command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RecoveryScope {
    /// All automatic recovery (the single recovery-intent domain).
    AllRecovery,
}

/// The scope of a configuration change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ConfigScope {
    All,
}

/// The normalized availability of a network snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SemanticPath {
    Unknown,
    Online,
    Offline,
}

/// The origin of a normalized signaling fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SignalingOrigin {
    /// Produced by the running lifecycle effect (its `action_id` and captured
    /// revision); recognized as the effect's own covered output.
    CurrentEffect { action_id: u64 },
    /// External news from a resource owner.
    External,
}

/// Why a signaling generation was lost. Diagnostic; does not affect the ruling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SignalingLostCause {
    Disconnected,
    Superseded,
    RemoteReset,
}

// ---------------------------------------------------------------------------
// Extended state carried by the view
// ---------------------------------------------------------------------------

/// One pending-work record's extended context (the RFC `RetryGateContext` plus
/// the record's kind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingRecord {
    /// The revision that most recently required this work.
    pub work_revision: Revision,
    pub gate: RetryGateState,
    /// Consecutive failures of the current work (0 while fresh).
    pub attempt: u32,
    /// The identity of the one armed backoff deadline, when backing off.
    pub retry_id: u64,
    /// Present exactly while the gate is `Parked`; never empty then.
    pub release_mask: ReleaseMask,
    /// For the recovery domain, which intent strength is pending.
    pub strength: Option<RecoveryStrength>,
}

impl PendingRecord {
    /// A fresh `Ready` recovery record for the given strength.
    pub(crate) fn recovery(work_revision: Revision, strength: RecoveryStrength) -> Self {
        Self {
            work_revision,
            gate: RetryGateState::Ready,
            attempt: 0,
            retry_id: 0,
            release_mask: ReleaseMask::default(),
            strength: Some(strength),
        }
    }

    /// A fresh `Ready` teardown record (cleanup or offline disconnect).
    pub(crate) fn teardown(work_revision: Revision) -> Self {
        Self {
            work_revision,
            gate: RetryGateState::Ready,
            attempt: 0,
            retry_id: 0,
            release_mask: ReleaseMask::default(),
            strength: None,
        }
    }
}

/// The currently running effect's context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EffectContext {
    pub action_id: u64,
    pub kind: EffectKind,
    pub captured_revision: Revision,
    pub cancel_reason: Option<CancelReason>,
}

/// The last accepted network snapshot's identity and semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AcceptedSnapshot {
    pub source_epoch: u64,
    pub sequence: u64,
    pub semantic_path: SemanticPath,
    pub route_fingerprint: u64,
}

/// The offline-candidate identity and its committed grace deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OfflineCandidate {
    pub candidate_id: u64,
    pub deadline: PolicyInstant,
}

/// An overall teardown deadline: armed once when the obligation is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TeardownDeadline {
    pub deadline_id: u64,
    pub deadline: PolicyInstant,
}

/// The shutdown overall deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ShutdownDeadline {
    pub deadline_id: u64,
    pub deadline: PolicyInstant,
}

/// The constructor-time choice of whether app phase gates recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LifecycleProfile {
    Ungated,
    Gated,
}

/// The supervisor's complete policy snapshot handed to [`translate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct View {
    pub recovery_mode: RecoveryModeState,
    pub app_phase: AppPhaseState,
    pub network_path: NetworkPathState,
    pub recovery_intent: RecoveryIntentState,
    pub cleanup_work: CleanupWorkState,
    pub offline_work: OfflineWorkState,
    pub execution: ExecutionState,

    pub recovery_record: Option<PendingRecord>,
    pub cleanup_record: Option<PendingRecord>,
    pub offline_record: Option<PendingRecord>,

    pub effect: Option<EffectContext>,

    pub policy_revision: Revision,
    pub last_snapshot: Option<AcceptedSnapshot>,
    pub offline_candidate: Option<OfflineCandidate>,
    pub cleanup_teardown: Option<TeardownDeadline>,
    pub offline_teardown: Option<TeardownDeadline>,
    pub background_entered_at: Option<PolicyInstant>,
    pub committed_session_generation: Option<Generation>,
    pub live_signaling_generation: Option<Generation>,
    /// Generations circled by a pending or running teardown scope; a generation
    /// here does not count as live for restore derivation.
    pub teardown_scope_generations: BTreeSet<Generation>,
    pub shutdown_deadline: Option<ShutdownDeadline>,
    pub profile: LifecycleProfile,
}

impl View {
    /// A benign initial snapshot: active, ungated, path unknown, nothing pending.
    pub(crate) fn initial() -> Self {
        Self {
            recovery_mode: RecoveryModeState::Active,
            app_phase: AppPhaseState::Unknown,
            network_path: NetworkPathState::Unknown,
            recovery_intent: RecoveryIntentState::Idle,
            cleanup_work: CleanupWorkState::Idle,
            offline_work: OfflineWorkState::Idle,
            execution: ExecutionState::Idle,
            recovery_record: None,
            cleanup_record: None,
            offline_record: None,
            effect: None,
            policy_revision: 0,
            last_snapshot: None,
            offline_candidate: None,
            cleanup_teardown: None,
            offline_teardown: None,
            background_entered_at: None,
            committed_session_generation: None,
            live_signaling_generation: None,
            teardown_scope_generations: BTreeSet::new(),
            shutdown_deadline: None,
            profile: LifecycleProfile::Ungated,
        }
    }

    fn current_recovery_strength(&self) -> Option<RecoveryStrength> {
        match self.recovery_intent {
            RecoveryIntentState::Idle => None,
            RecoveryIntentState::ProbePending => Some(RecoveryStrength::Probe),
            RecoveryIntentState::RestorePending => Some(RecoveryStrength::Restore),
            RecoveryIntentState::ReconnectPending => Some(RecoveryStrength::Reconnect),
        }
    }

    pub(crate) fn record(&self, domain: RetryDomain) -> Option<&PendingRecord> {
        match domain {
            RetryDomain::Recovery => self.recovery_record.as_ref(),
            RetryDomain::Cleanup => self.cleanup_record.as_ref(),
            RetryDomain::Offline => self.offline_record.as_ref(),
        }
    }

    /// Whether a live signaling generation exists *outside* any pending or
    /// running teardown scope. A generation circled by teardown does not count.
    fn live_signaling_outside_teardown(&self) -> bool {
        match self.live_signaling_generation {
            Some(g) => !self.teardown_scope_generations.contains(&g),
            None => false,
        }
    }

    fn phase_eligible(&self) -> bool {
        match self.profile {
            LifecycleProfile::Ungated => true,
            LifecycleProfile::Gated => self.app_phase == AppPhaseState::Foreground,
        }
    }
}

// ---------------------------------------------------------------------------
// Input event model
// ---------------------------------------------------------------------------

/// The full RFC-0400 supervisor input model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Input {
    AppEnteredForeground,
    AppEnteredBackground,
    SessionActivated {
        session_generation: Generation,
    },
    NetworkSnapshot {
        source_epoch: u64,
        sequence: u64,
        semantic_path: SemanticPath,
        route_fingerprint: u64,
    },
    RecoveryRequested {
        minimum: RecoveryStrength,
        reason: RecoveryRequestReason,
    },
    RecoveryPauseRequested {
        scope: RecoveryScope,
    },
    RecoveryResumeRequested {
        scope: RecoveryScope,
    },
    CleanupRequested {
        reason: CleanupReason,
    },
    ConfigurationChanged {
        scope: ConfigScope,
    },
    OfflineGraceExpired {
        candidate_id: u64,
    },
    RetryDeadlineExpired {
        domain: RetryDomain,
        work_revision: Revision,
        retry_id: u64,
    },
    BootstrapPhaseDeadlineExpired,
    ShutdownDeadlineExpired {
        deadline_id: u64,
    },
    TeardownDeadlineExpired {
        domain: TeardownDomain,
        deadline_id: u64,
    },
    SignalingGenerationCommitted {
        generation: Generation,
        origin: SignalingOrigin,
    },
    SignalingGenerationLost {
        generation: Generation,
        cause: SignalingLostCause,
    },
    EffectCompleted {
        action_id: u64,
        kind: EffectKind,
        policy_revision: Revision,
        outcome: EffectOutcome,
    },
    ShutdownRequested,
}

// ---------------------------------------------------------------------------
// Decision output
// ---------------------------------------------------------------------------

/// Whether the input allocates the next `policy_revision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RevisionDirective {
    /// The input materially changed a fact, gate, or pending-work domain.
    Advances,
    /// A structural duplicate or discarded input: no new revision.
    Unchanged,
}

/// A retry-gate trigger dispatch (the RFC "Trigger" column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GateTrigger {
    /// Apply `NewMaterialTrigger` to the domain's gate when it is `BackingOff`,
    /// waking it to `Ready` with a reset failure count.
    Wake { domain: RetryDomain },
    /// Clear release-mask entries matching `trigger` on a `Parked` record; the
    /// record is freed (via `NewMaterialTrigger`) only when the mask empties.
    ClearMask {
        domain: RetryDomain,
        trigger: TriggerClass,
    },
}

/// Why the supervisor requested cancellation of the running effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelReason {
    Pause,
    PreemptedByCleanup,
    PreemptedByStronger,
    PathRecovered,
    Shutdown,
}

/// A signaling-layer directive the shell executes after applying a decision.
///
/// Automatic-reconnect suppression and resumption are policy decisions, so they
/// are derived here by the one pure translation function rather than sniffed
/// from the raw event ahead of translation. The shell lowers each directive to
/// the signaling client through the `NetworkEventProcessor` hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignalingDirective {
    /// Invalidate in-flight automatic reconnect attempts and keep new attempts
    /// paused until a recovery path explicitly schedules them again.
    SuppressAutoReconnect,
    /// Re-enable future automatic reconnects without starting one immediately.
    ResumeAutoReconnect,
}

/// A category from the audited timer inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerCategory {
    BusinessHysteresis,
    FailureDeadline,
    FailureBackoff,
}

/// A stable timer inventory id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TimerId {
    OfflineCandidate,
    ShutdownOverall,
    TeardownOverall(TeardownDomain),
    BootstrapPhase,
    FailureBackoff(RetryDomain),
}

/// An arm/cancel instruction for the audited timer facade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerDirective {
    Arm {
        id: TimerId,
        category: TimerCategory,
        deadline: PolicyInstant,
    },
    Cancel {
        id: TimerId,
    },
}

/// Record the release mask that a `TerminalFailure`/`ExplicitPause` records on
/// a gate as it parks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParkDirective {
    pub domain: RetryDomain,
    pub release_mask: ReleaseMask,
}

/// Why a `RecoveryRequested` command was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RejectReason {
    LoggedOutOrTerminating,
    ParkedNoClearingTrigger,
}

/// A structured status-stream record produced by translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusRecord {
    RecoveryRejected {
        mode: RecoveryModeState,
        reason: RejectReason,
    },
    /// The gated bootstrap deadline elapsed with no authoritative phase.
    BootstrapDeadlineElapsed,
    /// The shutdown overall deadline aborted remaining work with `Abandoned`
    /// residuals; the supervisor ends unconditionally.
    ShutdownAbandon,
}

/// The result of translating one input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Decision {
    /// Ordered machine inputs to apply.
    pub machine_inputs: Vec<MachineInput>,
    /// Whether the input advances `policy_revision`.
    pub revision: RevisionDirective,
    /// Retry-gate trigger dispatch.
    pub gate_triggers: Vec<GateTrigger>,
    /// Release masks to record as records park.
    pub parks: Vec<ParkDirective>,
    /// Effect cancellation / preemption requests.
    pub cancels: Vec<CancelReason>,
    /// Audited timer arm/cancel instructions.
    pub timers: Vec<TimerDirective>,
    /// Structured status-stream records.
    pub status: Vec<StatusRecord>,
    /// Signaling-layer directives the shell executes (auto-reconnect control).
    pub signals: Vec<SignalingDirective>,
    /// The supervisor has ended unconditionally (its shutdown overall deadline
    /// aborted all remaining work); the shell must stop its reconcile loop.
    pub terminate: bool,
}

impl Decision {
    fn none() -> Self {
        Self {
            machine_inputs: Vec::new(),
            revision: RevisionDirective::Unchanged,
            gate_triggers: Vec::new(),
            parks: Vec::new(),
            cancels: Vec::new(),
            timers: Vec::new(),
            status: Vec::new(),
            signals: Vec::new(),
            terminate: false,
        }
    }

    fn advancing() -> Self {
        let mut d = Self::none();
        d.revision = RevisionDirective::Advances;
        d
    }

    fn machine(&mut self, input: MachineInput) -> &mut Self {
        self.machine_inputs.push(input);
        self
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Per-kind backoff curves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BackoffTable {
    pub probe: BackoffParams,
    pub restore: BackoffParams,
    pub reconnect: BackoffParams,
    pub teardown: BackoffParams,
}

impl BackoffTable {
    fn for_kind(&self, kind: EffectKind) -> &BackoffParams {
        match kind {
            EffectKind::Probe => &self.probe,
            EffectKind::Restore => &self.restore,
            EffectKind::Reconnect => &self.reconnect,
            EffectKind::ConfirmedOfflineDisconnect | EffectKind::Cleanup => &self.teardown,
        }
    }
}

/// Product policy constants used by translation; measured by telemetry rather
/// than part of the state model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PolicyConfig {
    pub offline_grace: Duration,
    pub background_reconnect_after: Duration,
    pub shutdown_deadline: Duration,
    pub cleanup_teardown_deadline: Duration,
    pub offline_teardown_deadline: Duration,
    pub bootstrap_phase_deadline: Duration,
    pub escalate_after_probe: u32,
    pub escalate_after_restore: u32,
    pub backoff: BackoffTable,
}

impl PolicyConfig {
    /// The documented defaults.
    pub(crate) fn defaults() -> Self {
        let curve = BackoffParams {
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            exponent_cap: 6,
            jitter_fraction: 0.2,
        };
        let teardown = BackoffParams {
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
            exponent_cap: 4,
            jitter_fraction: 0.2,
        };
        Self {
            offline_grace: Duration::from_millis(400),
            background_reconnect_after: Duration::from_secs(60),
            shutdown_deadline: Duration::from_secs(10),
            cleanup_teardown_deadline: Duration::from_secs(10),
            offline_teardown_deadline: Duration::from_secs(10),
            bootstrap_phase_deadline: Duration::from_secs(5),
            escalate_after_probe: 2,
            escalate_after_restore: 2,
            backoff: BackoffTable {
                probe: curve,
                restore: curve,
                reconnect: curve,
                teardown,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// translate
// ---------------------------------------------------------------------------

/// Translate one accepted input into a [`Decision`].
///
/// The rows of the RFC translation table are grouped by input; within a group
/// they are evaluated top to bottom and the first matching row applies. Entropy
/// is drawn only when a `Retry` verdict arms a backoff deadline; every other
/// input leaves the entropy stream untouched, so duplicates never resample.
pub(crate) fn translate(
    view: &View,
    input: &Input,
    now: PolicyInstant,
    config: &PolicyConfig,
    entropy: &mut dyn EntropySource,
) -> Decision {
    match input {
        Input::AppEnteredForeground => translate_foreground(view, now, config),
        Input::AppEnteredBackground => translate_background(view),
        Input::SessionActivated { session_generation } => {
            translate_session_activated(view, *session_generation)
        }
        Input::NetworkSnapshot {
            source_epoch,
            sequence,
            semantic_path,
            route_fingerprint,
        } => translate_snapshot(
            view,
            *source_epoch,
            *sequence,
            *semantic_path,
            *route_fingerprint,
            now,
            config,
        ),
        Input::RecoveryRequested { minimum, .. } => translate_recovery_requested(view, *minimum),
        Input::RecoveryPauseRequested { .. } => translate_pause(view),
        Input::RecoveryResumeRequested { .. } => translate_resume(view),
        Input::CleanupRequested { reason } => translate_cleanup(view, *reason, now, config),
        Input::ConfigurationChanged { .. } => translate_configuration_changed(),
        Input::OfflineGraceExpired { candidate_id } => {
            translate_offline_grace_expired(view, *candidate_id, now, config)
        }
        Input::RetryDeadlineExpired {
            domain,
            work_revision,
            retry_id,
        } => translate_retry_deadline_expired(view, *domain, *work_revision, *retry_id),
        Input::BootstrapPhaseDeadlineExpired => translate_bootstrap_deadline(view),
        Input::ShutdownDeadlineExpired { deadline_id } => {
            translate_shutdown_deadline(view, *deadline_id)
        }
        Input::TeardownDeadlineExpired {
            domain,
            deadline_id,
        } => translate_teardown_deadline(view, *domain, *deadline_id),
        Input::SignalingGenerationCommitted { generation, origin } => {
            translate_signaling_committed(view, *generation, *origin)
        }
        Input::SignalingGenerationLost { generation, .. } => {
            translate_signaling_lost(view, *generation)
        }
        Input::EffectCompleted {
            action_id,
            kind,
            policy_revision,
            outcome,
        } => translate_effect_completed(
            view,
            *action_id,
            *kind,
            *policy_revision,
            outcome,
            now,
            config,
            entropy,
        ),
        Input::ShutdownRequested => {
            translate_cleanup(view, CleanupReason::AppTerminating, now, config)
        }
    }
}

fn translate_foreground(view: &View, now: PolicyInstant, config: &PolicyConfig) -> Decision {
    // Row 1: mode LoggedOut/Terminating -> phase only, gated.
    if matches!(
        view.recovery_mode,
        RecoveryModeState::LoggedOut | RecoveryModeState::Terminating
    ) {
        let mut d = Decision::advancing();
        d.machine(MachineInput::AppPhase(AppPhaseInput::EnterForeground));
        return d;
    }
    // mode Active
    match view.app_phase {
        // Row 4: already Foreground -> self-loop, no revision.
        AppPhaseState::Foreground => Decision::none(),
        // Row 2: from Background -> derive probe or reconnect by elapsed time.
        AppPhaseState::Background => {
            let mut d = Decision::advancing();
            d.machine(MachineInput::AppPhase(AppPhaseInput::EnterForeground));
            let elapsed = now.saturating_sub(view.background_entered_at.unwrap_or(now));
            if elapsed < config.background_reconnect_after {
                // Short background: probe the possibly-healthy socket and let
                // automatic reconnect resume rather than force a rebuild.
                d.machine(MachineInput::RecoveryIntent(
                    RecoveryIntentInput::RequestProbe,
                ));
                d.signals.push(SignalingDirective::ResumeAutoReconnect);
            } else {
                // Long background: force a reconnect and keep any stale automatic
                // attempt suppressed until that fresh connect path re-enables it.
                d.machine(MachineInput::RecoveryIntent(
                    RecoveryIntentInput::RequestReconnect,
                ));
                d.signals.push(SignalingDirective::SuppressAutoReconnect);
            }
            // Wakes records backing off after a phase-ineligible failure; parks
            // remain until their masks clear (foreground is not a mask trigger).
            d.gate_triggers.push(GateTrigger::Wake {
                domain: RetryDomain::Recovery,
            });
            d
        }
        // Row 3: first authoritative phase from Unknown.
        AppPhaseState::Unknown => {
            let mut d = Decision::advancing();
            d.machine(MachineInput::AppPhase(AppPhaseInput::EnterForeground));
            d.gate_triggers.push(GateTrigger::Wake {
                domain: RetryDomain::Recovery,
            });
            d
        }
    }
}

fn translate_background(view: &View) -> Decision {
    // Row 2: already Background -> self-loop, no revision.
    if view.app_phase == AppPhaseState::Background {
        return Decision::none();
    }
    // Row 1: enter background; the shell stamps background_entered_at = now.
    let mut d = Decision::advancing();
    d.machine(MachineInput::AppPhase(AppPhaseInput::EnterBackground));
    // Entering the background pauses stale automatic reconnect without
    // disconnecting a healthy socket (derived here, not sniffed pre-translation).
    d.signals.push(SignalingDirective::SuppressAutoReconnect);
    d
}

fn translate_session_activated(view: &View, generation: Generation) -> Decision {
    let newer = view
        .committed_session_generation
        .is_none_or(|committed| generation > committed);
    if !newer {
        // Row 2: not newer -> nothing.
        return Decision::none();
    }
    // Row 1: commit the new session at revision r.
    let mut d = Decision::advancing();
    d.machine(MachineInput::RecoveryMode(
        RecoveryModeInput::SessionActivated,
    ));
    // Supersede old-session recovery intent (revision < r).
    if view.recovery_intent != RecoveryIntentState::Idle {
        d.machine(MachineInput::RecoveryIntent(
            RecoveryIntentInput::SupersedeRecovery,
        ));
    }
    // Derive restoration when no live generation exists outside a teardown
    // scope; a generation circled by a pending cleanup does not count as live.
    if !view.live_signaling_outside_teardown() {
        d.machine(MachineInput::RecoveryIntent(
            RecoveryIntentInput::RequestRestore,
        ));
    }
    // Clears SessionActivated mask entries (auth parks); material for recovery.
    d.gate_triggers.push(GateTrigger::ClearMask {
        domain: RetryDomain::Recovery,
        trigger: TriggerClass::SessionActivated,
    });
    d
}

#[allow(clippy::too_many_arguments)]
fn translate_snapshot(
    view: &View,
    source_epoch: u64,
    sequence: u64,
    semantic_path: SemanticPath,
    route_fingerprint: u64,
    now: PolicyInstant,
    config: &PolicyConfig,
) -> Decision {
    // Acceptance by (epoch, sequence).
    let accepted = match &view.last_snapshot {
        None => true,
        Some(last) => match source_epoch.cmp(&last.source_epoch) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => sequence > last.sequence,
        },
    };
    if !accepted {
        // Stale (epoch, sequence) -> discarded.
        return Decision::none();
    }
    // Structural duplicate: accepted by sequence but semantically equal. A
    // route fingerprint is only meaningful while `Online`; a route change under
    // an offline or unknown path is not a routing change and must not re-arm the
    // candidate.
    let material = view.last_snapshot.is_none_or(|last| {
        last.semantic_path != semantic_path
            || (semantic_path == SemanticPath::Online
                && last.route_fingerprint != route_fingerprint)
    });
    if !material {
        return Decision::none();
    }

    match semantic_path {
        SemanticPath::Online => {
            if view.network_path == NetworkPathState::Online {
                // Online -> Online material route change: self-loop + trigger.
                let mut d = Decision::advancing();
                d.gate_triggers.push(GateTrigger::Wake {
                    domain: RetryDomain::Recovery,
                });
                d
            } else {
                let mut d = Decision::advancing();
                d.machine(MachineInput::NetworkPath(NetworkPathInput::ObserveOnline));
                d.timers.push(TimerDirective::Cancel {
                    id: TimerId::OfflineCandidate,
                });
                if view.offline_work == OfflineWorkState::DisconnectPending {
                    d.machine(MachineInput::OfflineWork(
                        OfflineWorkInput::SupersedeDisconnect,
                    ));
                }
                if view.execution == ExecutionState::Disconnecting {
                    d.cancels.push(CancelReason::PathRecovered);
                }
                if view.recovery_mode == RecoveryModeState::Active
                    && view.recovery_intent == RecoveryIntentState::Idle
                {
                    let derived = if view.live_signaling_generation.is_some() {
                        RecoveryIntentInput::RequestProbe
                    } else {
                        RecoveryIntentInput::RequestRestore
                    };
                    d.machine(MachineInput::RecoveryIntent(derived));
                }
                d.gate_triggers.push(GateTrigger::Wake {
                    domain: RetryDomain::Recovery,
                });
                d
            }
        }
        SemanticPath::Offline => {
            // Prior path is Unknown or Online (Offline -> Offline is a dup).
            let mut d = Decision::advancing();
            d.machine(MachineInput::NetworkPath(NetworkPathInput::ObserveOffline));
            d.timers.push(TimerDirective::Arm {
                id: TimerId::OfflineCandidate,
                category: TimerCategory::BusinessHysteresis,
                deadline: now + config.offline_grace,
            });
            d
        }
        SemanticPath::Unknown => {
            let mut d = Decision::advancing();
            d.machine(MachineInput::NetworkPath(NetworkPathInput::ObserveUnknown));
            d.timers.push(TimerDirective::Cancel {
                id: TimerId::OfflineCandidate,
            });
            if view.offline_work == OfflineWorkState::DisconnectPending {
                d.machine(MachineInput::OfflineWork(
                    OfflineWorkInput::SupersedeDisconnect,
                ));
            }
            if view.execution == ExecutionState::Disconnecting {
                d.cancels.push(CancelReason::PathRecovered);
            }
            d
        }
    }
}

fn translate_recovery_requested(view: &View, minimum: RecoveryStrength) -> Decision {
    // Row 1: logged out or terminating -> structured rejection.
    if matches!(
        view.recovery_mode,
        RecoveryModeState::LoggedOut | RecoveryModeState::Terminating
    ) {
        let mut d = Decision::none();
        d.status.push(StatusRecord::RecoveryRejected {
            mode: view.recovery_mode,
            reason: RejectReason::LoggedOutOrTerminating,
        });
        return d;
    }
    let current = view.current_recovery_strength();
    // Row 2: idle or pending weaker than minimum -> request the stronger work.
    if current.is_none_or(|c| c < minimum) {
        let mut d = Decision::advancing();
        d.machine(MachineInput::RecoveryIntent(minimum.request_input()));
        // A stronger requirement preempts a weaker recovery effect already
        // running, per the RFC effect-preemption table; the stronger action
        // starts once the single-flight slot returns to Idle.
        if let Some(running) = executing_recovery_strength(view.execution)
            && running < minimum
        {
            d.cancels.push(CancelReason::PreemptedByStronger);
        }
        return d;
    }
    // pending >= minimum
    let gate = view
        .recovery_record
        .as_ref()
        .map_or(RetryGateState::Ready, |r| r.gate);
    match gate {
        // Row 3: gate Ready or effect running -> coalesced.
        RetryGateState::Ready => Decision::none(),
        // Row 4: gate BackingOff -> deliberate retry.
        RetryGateState::BackingOff => {
            let mut d = Decision::advancing();
            d.gate_triggers.push(GateTrigger::Wake {
                domain: RetryDomain::Recovery,
            });
            d
        }
        // Rows 5-6: gate Parked -> release only if the command clears an entry.
        RetryGateState::Parked => {
            let clears = view
                .recovery_record
                .as_ref()
                .is_some_and(|r| r.release_mask.would_clear(TriggerClass::ExplicitCommand));
            if clears {
                let mut d = Decision::advancing();
                d.gate_triggers.push(GateTrigger::ClearMask {
                    domain: RetryDomain::Recovery,
                    trigger: TriggerClass::ExplicitCommand,
                });
                d
            } else {
                let mut d = Decision::none();
                d.status.push(StatusRecord::RecoveryRejected {
                    mode: view.recovery_mode,
                    reason: RejectReason::ParkedNoClearingTrigger,
                });
                d
            }
        }
    }
}

fn translate_pause(view: &View) -> Decision {
    // Row 1: a scoped record's effect is in flight -> request pause cancellation.
    if view.execution.is_recovery() {
        let mut d = Decision::advancing();
        d.cancels.push(CancelReason::Pause);
        return d;
    }
    // Row 2: scoped recovery records exist, none executing -> pause the gate.
    if view.recovery_record.is_some() {
        let mut d = Decision::advancing();
        d.machine(MachineInput::RetryGate {
            domain: RetryDomain::Recovery,
            input: RetryGateInput::ExplicitPause,
        });
        d.parks.push(ParkDirective {
            domain: RetryDomain::Recovery,
            release_mask: ReleaseMask::single(super::classification::ParkCause::ExplicitPause),
        });
        d.timers.push(TimerDirective::Cancel {
            id: TimerId::FailureBackoff(RetryDomain::Recovery),
        });
        return d;
    }
    Decision::none()
}

fn translate_resume(view: &View) -> Decision {
    // Paused records in scope -> clear ExplicitPause entries.
    let paused = view
        .recovery_record
        .as_ref()
        .is_some_and(|r| r.release_mask.would_clear(TriggerClass::ExplicitResume));
    if paused {
        let mut d = Decision::advancing();
        d.gate_triggers.push(GateTrigger::ClearMask {
            domain: RetryDomain::Recovery,
            trigger: TriggerClass::ExplicitResume,
        });
        return d;
    }
    Decision::none()
}

fn translate_configuration_changed() -> Decision {
    // Any: clears ConfigurationChanged mask entries; material for recovery.
    let mut d = Decision::advancing();
    d.gate_triggers.push(GateTrigger::ClearMask {
        domain: RetryDomain::Recovery,
        trigger: TriggerClass::ConfigurationChanged,
    });
    d.gate_triggers.push(GateTrigger::Wake {
        domain: RetryDomain::Recovery,
    });
    d
}

fn translate_cleanup(
    view: &View,
    reason: CleanupReason,
    now: PolicyInstant,
    config: &PolicyConfig,
) -> Decision {
    let mut d = Decision::advancing();
    // Mode input by reason.
    match reason {
        CleanupReason::UserLogout => {
            d.machine(MachineInput::RecoveryMode(RecoveryModeInput::UserLoggedOut));
        }
        CleanupReason::AppTerminating => {
            d.machine(MachineInput::RecoveryMode(
                RecoveryModeInput::AppTerminating,
            ));
            d.timers.push(TimerDirective::Arm {
                id: TimerId::ShutdownOverall,
                category: TimerCategory::FailureDeadline,
                deadline: now + config.shutdown_deadline,
            });
        }
        CleanupReason::ManualReset | CleanupReason::StaleConnectionSuspected => {}
    }
    // Additive: an offline candidate is committed for policy purposes and
    // CleanupWork owns the teardown (no duplicate OfflineWork).
    if view.network_path == NetworkPathState::OfflineCandidate {
        d.timers.push(TimerDirective::Cancel {
            id: TimerId::OfflineCandidate,
        });
        d.machine(MachineInput::NetworkPath(NetworkPathInput::CommitOffline));
    }
    // Request cleanup at revision r and arm its overall teardown deadline.
    d.machine(MachineInput::CleanupWork(CleanupWorkInput::RequestCleanup));
    d.timers.push(TimerDirective::Arm {
        id: TimerId::TeardownOverall(TeardownDomain::Cleanup),
        category: TimerCategory::FailureDeadline,
        deadline: now + config.cleanup_teardown_deadline,
    });
    // Supersede recovery intent and offline work with revision <= r.
    if view.recovery_intent != RecoveryIntentState::Idle {
        d.machine(MachineInput::RecoveryIntent(
            RecoveryIntentInput::SupersedeRecovery,
        ));
    }
    if view.offline_work == OfflineWorkState::DisconnectPending {
        d.machine(MachineInput::OfflineWork(
            OfflineWorkInput::SupersedeDisconnect,
        ));
    }
    // Preempt a running non-cleanup effect.
    if let Some(effect) = &view.effect
        && effect.kind != EffectKind::Cleanup
    {
        d.cancels.push(CancelReason::PreemptedByCleanup);
    }
    d
}

fn translate_offline_grace_expired(
    view: &View,
    candidate_id: u64,
    now: PolicyInstant,
    config: &PolicyConfig,
) -> Decision {
    let current = view
        .offline_candidate
        .is_some_and(|c| c.candidate_id == candidate_id);
    if !current {
        // Stale candidate -> logged, nothing.
        return Decision::none();
    }
    let mut d = Decision::advancing();
    d.machine(MachineInput::NetworkPath(NetworkPathInput::CommitOffline));
    d.machine(MachineInput::OfflineWork(
        OfflineWorkInput::RequestDisconnect,
    ));
    // Arm the offline-disconnect overall teardown deadline once at creation.
    d.timers.push(TimerDirective::Arm {
        id: TimerId::TeardownOverall(TeardownDomain::OfflineDisconnect),
        category: TimerCategory::FailureDeadline,
        deadline: now + config.offline_teardown_deadline,
    });
    d
}

fn translate_retry_deadline_expired(
    view: &View,
    domain: RetryDomain,
    work_revision: Revision,
    retry_id: u64,
) -> Decision {
    let matches = view.record(domain).is_some_and(|r| {
        r.gate == RetryGateState::BackingOff
            && r.work_revision == work_revision
            && r.retry_id == retry_id
    });
    if !matches {
        return Decision::none();
    }
    let mut d = Decision::advancing();
    d.machine(MachineInput::RetryGate {
        domain,
        input: RetryGateInput::RetryDeadlineExpired,
    });
    d
}

fn translate_bootstrap_deadline(view: &View) -> Decision {
    if view.profile == LifecycleProfile::Gated && view.app_phase == AppPhaseState::Unknown {
        let mut d = Decision::none();
        d.status.push(StatusRecord::BootstrapDeadlineElapsed);
        return d;
    }
    Decision::none()
}

fn translate_shutdown_deadline(view: &View, deadline_id: u64) -> Decision {
    let matches = view.recovery_mode == RecoveryModeState::Terminating
        && view
            .shutdown_deadline
            .is_some_and(|s| s.deadline_id == deadline_id);
    if !matches {
        return Decision::none();
    }
    let mut d = Decision::none();
    // Abort the in-flight effect and record Abandoned residuals.
    if view.execution != ExecutionState::Idle {
        d.cancels.push(CancelReason::Shutdown);
    }
    // Detach any still-pending teardown obligation with Abandoned semantics so a
    // lingering record cannot restart cleanup or disconnect after the deadline.
    if view.cleanup_work == CleanupWorkState::CleanupPending {
        d.machine(MachineInput::CleanupWork(CleanupWorkInput::CompleteCleanup));
    }
    if view.offline_work == OfflineWorkState::DisconnectPending {
        d.machine(MachineInput::OfflineWork(
            OfflineWorkInput::CompleteDisconnect,
        ));
    }
    d.status.push(StatusRecord::ShutdownAbandon);
    // The supervisor ends unconditionally (RFC-0400 invariant 30); the shell
    // stops its reconcile loop instead of re-deriving cleanup.
    d.terminate = true;
    d
}

fn translate_teardown_deadline(view: &View, domain: TeardownDomain, deadline_id: u64) -> Decision {
    let (retry_domain, complete, running, deadline) = match domain {
        TeardownDomain::Cleanup => (
            RetryDomain::Cleanup,
            MachineInput::CleanupWork(CleanupWorkInput::CompleteCleanup),
            view.execution == ExecutionState::Cleaning,
            view.cleanup_teardown,
        ),
        TeardownDomain::OfflineDisconnect => (
            RetryDomain::Offline,
            MachineInput::OfflineWork(OfflineWorkInput::CompleteDisconnect),
            view.execution == ExecutionState::Disconnecting,
            view.offline_teardown,
        ),
    };
    // Row 2: stale, or the teardown effect is running (in-effect expiry applies).
    let matches = !running
        && deadline.is_some_and(|t| t.deadline_id == deadline_id)
        && view
            .record(retry_domain)
            .is_some_and(|r| r.gate == RetryGateState::BackingOff);
    if !matches {
        return Decision::none();
    }
    // Row 1: extinguish the obligation with Abandoned residuals.
    let mut d = Decision::advancing();
    d.machine(complete);
    d.timers.push(TimerDirective::Cancel {
        id: TimerId::TeardownOverall(domain),
    });
    d
}

fn translate_signaling_committed(
    view: &View,
    generation: Generation,
    origin: SignalingOrigin,
) -> Decision {
    let is_current = match origin {
        SignalingOrigin::CurrentEffect { action_id } => view
            .effect
            .as_ref()
            .is_some_and(|e| e.action_id == action_id),
        SignalingOrigin::External => false,
    };
    if is_current {
        // Row 1: the current effect's covered output; record live, no trigger.
        return Decision::advancing();
    }
    // Row 2: external; only newer generations are material.
    if view
        .live_signaling_generation
        .is_none_or(|cur| generation > cur)
    {
        let mut d = Decision::advancing();
        d.gate_triggers.push(GateTrigger::Wake {
            domain: RetryDomain::Recovery,
        });
        return d;
    }
    Decision::none()
}

fn translate_signaling_lost(view: &View, generation: Generation) -> Decision {
    // Row 1: stale generation -> logged, nothing.
    if view.live_signaling_generation != Some(generation) {
        return Decision::none();
    }
    // Row 2: live; mode Active, cleanup idle and no cleanup effect -> restore.
    if view.recovery_mode == RecoveryModeState::Active
        && view.cleanup_work == CleanupWorkState::Idle
        && view.execution != ExecutionState::Cleaning
    {
        let mut d = Decision::advancing();
        d.machine(MachineInput::RecoveryIntent(
            RecoveryIntentInput::RequestRestore,
        ));
        d.gate_triggers.push(GateTrigger::Wake {
            domain: RetryDomain::Recovery,
        });
        return d;
    }
    // Row 3: live but absorbed by cleanup or gated by mode -> record not live.
    Decision::advancing()
}

#[allow(clippy::too_many_arguments)]
fn translate_effect_completed(
    view: &View,
    action_id: u64,
    kind: EffectKind,
    policy_revision: Revision,
    outcome: &EffectOutcome,
    now: PolicyInstant,
    config: &PolicyConfig,
    entropy: &mut dyn EntropySource,
) -> Decision {
    // Accept only when action_id, kind, and captured revision all match.
    let matched = view.effect.as_ref().is_some_and(|e| {
        e.action_id == action_id && e.kind == kind && e.captured_revision == policy_revision
    });
    if !matched {
        // Stale or mismatched -> logged and discarded.
        return Decision::none();
    }
    // EffectCompleted never allocates a revision.
    let mut d = Decision::none();
    let domain = retry_domain_of(kind);

    match outcome {
        EffectOutcome::Succeeded
        | EffectOutcome::CompletedWithResiduals { .. }
        | EffectOutcome::Abandoned { .. } => {
            d.machine(MachineInput::Execution(ExecutionInput::Succeeded));
            acknowledge_success(view, kind, policy_revision, &mut d);
        }
        EffectOutcome::Failed { diagnosis } => {
            d.machine(MachineInput::Execution(ExecutionInput::Failed));
            // Only a still-current obligation receives a classification verdict.
            // A completion whose record was superseded or re-dispatched (its
            // record is gone, or its work revision is newer than this effect's
            // captured revision) releases the execution slot and nothing else.
            // This mirrors the acknowledgement guard so a stale failure cannot
            // back off, escalate, or resurrect newer work.
            if still_current_work(view, domain, policy_revision) {
                apply_failure(view, kind, domain, diagnosis, now, config, entropy, &mut d);
                apply_deferred_pause(view, domain, &mut d);
            }
        }
        EffectOutcome::Cancelled => {
            d.machine(MachineInput::Execution(ExecutionInput::Cancelled));
            apply_deferred_pause(view, domain, &mut d);
        }
        EffectOutcome::Aborted { cause } => {
            d.machine(MachineInput::Execution(ExecutionInput::Cancelled));
            // Same still-current guard as the failure path: a stale abort of
            // superseded work must not rule on the record that replaced it.
            if still_current_work(view, domain, policy_revision) {
                apply_abort(view, kind, domain, cause, &mut d);
                apply_deferred_pause(view, domain, &mut d);
            }
        }
    }
    d
}

fn retry_domain_of(kind: EffectKind) -> RetryDomain {
    match kind {
        EffectKind::Probe | EffectKind::Restore | EffectKind::Reconnect => RetryDomain::Recovery,
        EffectKind::Cleanup => RetryDomain::Cleanup,
        EffectKind::ConfirmedOfflineDisconnect => RetryDomain::Offline,
    }
}

/// Whether the obligation an effect served is still the current one: its record
/// is present and its work revision is not newer than the effect's captured
/// `policy_revision`. This is the same predicate as success acknowledgement, so
/// a stale failure, escalation, or abort cannot rule on newer or re-dispatched
/// work in the domain.
fn still_current_work(view: &View, domain: RetryDomain, policy_revision: Revision) -> bool {
    view.record(domain)
        .is_some_and(|r| r.work_revision <= policy_revision)
}

/// The recovery strength of the action currently occupying the execution slot,
/// or `None` when the slot is idle or running a teardown effect.
fn executing_recovery_strength(exec: ExecutionState) -> Option<RecoveryStrength> {
    match exec {
        ExecutionState::Probing => Some(RecoveryStrength::Probe),
        ExecutionState::Restoring => Some(RecoveryStrength::Restore),
        ExecutionState::Reconnecting => Some(RecoveryStrength::Reconnect),
        _ => None,
    }
}

fn acknowledge_success(view: &View, kind: EffectKind, policy_revision: Revision, d: &mut Decision) {
    match kind {
        EffectKind::Probe | EffectKind::Restore | EffectKind::Reconnect => {
            // Acknowledge only the current pending intent, and only when the
            // effect kind covers it and its work revision is not newer.
            let kind_strength = recovery_strength_of(kind);
            if let (Some(current), Some(rec)) =
                (view.current_recovery_strength(), &view.recovery_record)
                && kind_strength >= current
                && rec.work_revision <= policy_revision
            {
                d.machine(MachineInput::RecoveryIntent(current.complete_input()));
            }
        }
        EffectKind::ConfirmedOfflineDisconnect => {
            if view
                .offline_record
                .as_ref()
                .is_some_and(|r| r.work_revision <= policy_revision)
            {
                d.machine(MachineInput::OfflineWork(
                    OfflineWorkInput::CompleteDisconnect,
                ));
            }
        }
        EffectKind::Cleanup => {
            if view
                .cleanup_record
                .as_ref()
                .is_some_and(|r| r.work_revision <= policy_revision)
            {
                d.machine(MachineInput::CleanupWork(CleanupWorkInput::CompleteCleanup));
            }
            // Cleanup covers an older offline disconnect.
            if view
                .offline_record
                .as_ref()
                .is_some_and(|r| r.work_revision <= policy_revision)
            {
                d.machine(MachineInput::OfflineWork(
                    OfflineWorkInput::SupersedeDisconnect,
                ));
            }
        }
    }
}

fn recovery_strength_of(kind: EffectKind) -> RecoveryStrength {
    match kind {
        EffectKind::Probe => RecoveryStrength::Probe,
        EffectKind::Restore => RecoveryStrength::Restore,
        // Reconnect and (defensively) teardown kinds map to the strongest.
        _ => RecoveryStrength::Reconnect,
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_failure(
    view: &View,
    kind: EffectKind,
    domain: RetryDomain,
    diagnosis: &EffectDiagnosis,
    now: PolicyInstant,
    config: &PolicyConfig,
    entropy: &mut dyn EntropySource,
    d: &mut Decision,
) {
    let this_attempt = view
        .record(domain)
        .map_or(0, |r| r.attempt)
        .saturating_add(1);
    let ctx = ClassifyContext {
        attempt: this_attempt,
        path_online: view.network_path == NetworkPathState::Online,
        mode_terminating: view.recovery_mode == RecoveryModeState::Terminating,
        past_teardown_deadline: past_teardown_deadline(view, domain, now),
        escalate_after_probe: config.escalate_after_probe,
        escalate_after_restore: config.escalate_after_restore,
    };
    match classify(kind, diagnosis, ctx) {
        Verdict::Retry => {
            d.machine(MachineInput::RetryGate {
                domain,
                input: RetryGateInput::RetryableFailure,
            });
            let retry_after = diagnosis.retry_after().unwrap_or(Duration::ZERO);
            let deadline = arm_backoff(
                now,
                this_attempt,
                config.backoff.for_kind(kind),
                retry_after,
                entropy,
            );
            d.timers.push(TimerDirective::Arm {
                id: TimerId::FailureBackoff(domain),
                category: TimerCategory::FailureBackoff,
                deadline,
            });
        }
        Verdict::Escalate { to } => {
            // Promotion retains the causal work revision (shell detail) and
            // creates a fresh Ready gate; the replaced completion is not
            // dispatched.
            d.machine(MachineInput::RecoveryIntent(
                recovery_strength_of(to).request_input(),
            ));
        }
        Verdict::Park { release_mask } => {
            d.machine(MachineInput::RetryGate {
                domain,
                input: RetryGateInput::TerminalFailure,
            });
            d.parks.push(ParkDirective {
                domain,
                release_mask,
            });
        }
        Verdict::Abandon => {
            // Teardown obligation extinguished at once with Abandoned residuals.
            match kind {
                EffectKind::Cleanup => {
                    d.machine(MachineInput::CleanupWork(CleanupWorkInput::CompleteCleanup));
                }
                EffectKind::ConfirmedOfflineDisconnect => {
                    d.machine(MachineInput::OfflineWork(
                        OfflineWorkInput::CompleteDisconnect,
                    ));
                }
                _ => {}
            }
        }
    }
}

fn past_teardown_deadline(view: &View, domain: RetryDomain, now: PolicyInstant) -> bool {
    match domain {
        RetryDomain::Cleanup => view.cleanup_teardown.is_some_and(|t| now >= t.deadline),
        RetryDomain::Offline => view.offline_teardown.is_some_and(|t| now >= t.deadline),
        RetryDomain::Recovery => false,
    }
}

fn apply_abort(
    view: &View,
    kind: EffectKind,
    domain: RetryDomain,
    cause: &AbortCause,
    d: &mut Decision,
) {
    match cause {
        // Handled as an ordinary supervisor cancellation (retain work).
        AbortCause::SupervisorCancellation => {}
        // Ruled immediately as InvariantViolation's fail-safe verdict.
        AbortCause::PanicOrContractViolation | AbortCause::Unclassified => {
            if kind.is_teardown() && view.recovery_mode == RecoveryModeState::Terminating {
                // Terminating teardown invariant -> Abandon.
                match kind {
                    EffectKind::Cleanup => {
                        d.machine(MachineInput::CleanupWork(CleanupWorkInput::CompleteCleanup));
                    }
                    EffectKind::ConfirmedOfflineDisconnect => {
                        d.machine(MachineInput::OfflineWork(
                            OfflineWorkInput::CompleteDisconnect,
                        ));
                    }
                    _ => {}
                }
            } else {
                d.machine(MachineInput::RetryGate {
                    domain,
                    input: RetryGateInput::TerminalFailure,
                });
                d.parks.push(ParkDirective {
                    domain,
                    release_mask: ReleaseMask::single(
                        super::classification::ParkCause::InvariantViolation,
                    ),
                });
            }
        }
        // Runtime shutdown takes the Terminating bounded-teardown path.
        AbortCause::RuntimeShutdown => {
            if kind.is_teardown() {
                match kind {
                    EffectKind::Cleanup => {
                        d.machine(MachineInput::CleanupWork(CleanupWorkInput::CompleteCleanup));
                    }
                    EffectKind::ConfirmedOfflineDisconnect => {
                        d.machine(MachineInput::OfflineWork(
                            OfflineWorkInput::CompleteDisconnect,
                        ));
                    }
                    _ => {}
                }
            }
        }
    }
}

fn apply_deferred_pause(view: &View, domain: RetryDomain, d: &mut Decision) {
    // A recorded pause cancel reason applies its deferred ExplicitPause to the
    // surviving record in the same turn, before reconciliation.
    if view
        .effect
        .as_ref()
        .is_some_and(|e| e.cancel_reason == Some(CancelReason::Pause))
    {
        d.machine(MachineInput::RetryGate {
            domain,
            input: RetryGateInput::ExplicitPause,
        });
        d.parks.push(ParkDirective {
            domain,
            release_mask: ReleaseMask::single(super::classification::ParkCause::ExplicitPause),
        });
    }
}

// ---------------------------------------------------------------------------
// Stage 2: composite action decision and derived send projection
// ---------------------------------------------------------------------------

/// One executable action selected from the combined policy snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Cleanup,
    ConfirmedOfflineDisconnect,
    Reconnect,
    Restore,
    Probe,
}

fn gate_ready(record: &Option<PendingRecord>) -> bool {
    record
        .as_ref()
        .is_some_and(|r| r.gate == RetryGateState::Ready)
}

/// Derive at most one executable action, evaluating the composite decision
/// table top to bottom. Returns `None` when no action is currently admissible.
pub(crate) fn composite_action(view: &View) -> Option<Action> {
    // Cleanup shadows every lower domain, even while backing off or parked.
    if view.cleanup_work == CleanupWorkState::CleanupPending {
        return gate_ready(&view.cleanup_record).then_some(Action::Cleanup);
    }
    // Confirmed offline disconnect, only once the path has committed to Offline.
    if view.network_path == NetworkPathState::Offline
        && view.offline_work == OfflineWorkState::DisconnectPending
    {
        return gate_ready(&view.offline_record).then_some(Action::ConfirmedOfflineDisconnect);
    }
    // Recovery intent is gated by mode and phase eligibility.
    if matches!(
        view.recovery_mode,
        RecoveryModeState::LoggedOut | RecoveryModeState::Terminating
    ) {
        return None;
    }
    if !view.phase_eligible() {
        return None;
    }
    if matches!(
        view.network_path,
        NetworkPathState::OfflineCandidate | NetworkPathState::Offline
    ) {
        return None;
    }
    if !gate_ready(&view.recovery_record) {
        return None;
    }
    match view.recovery_intent {
        RecoveryIntentState::ReconnectPending => Some(Action::Reconnect),
        RecoveryIntentState::RestorePending => Some(Action::Restore),
        RecoveryIntentState::ProbePending => Some(Action::Probe),
        RecoveryIntentState::Idle => None,
    }
}

/// The read-only outbound-path admission projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendProjection {
    Blocked,
    ExistingOnly,
    Normal,
}

/// Project outbound send admission from the three authoritative fields. Rows
/// are evaluated top to bottom; the strictest match wins.
pub(crate) fn derive_send_policy(view: &View) -> SendProjection {
    let teardown_active = view.cleanup_work == CleanupWorkState::CleanupPending
        || view.offline_work == OfflineWorkState::DisconnectPending
        || matches!(
            view.execution,
            ExecutionState::Cleaning | ExecutionState::Disconnecting
        );
    if view.recovery_mode != RecoveryModeState::Active
        || teardown_active
        || view.network_path == NetworkPathState::Offline
    {
        return SendProjection::Blocked;
    }
    if view.network_path == NetworkPathState::OfflineCandidate {
        return SendProjection::ExistingOnly;
    }
    SendProjection::Normal
}

#[cfg(test)]
#[path = "translate_tests.rs"]
mod translate_tests;

#[cfg(test)]
#[path = "translate_bench.rs"]
mod translate_bench;
