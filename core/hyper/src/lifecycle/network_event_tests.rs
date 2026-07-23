use super::*;
use crate::lifecycle::CredentialState;
use crate::transport::{NetworkError, NetworkResult};
use crate::wire::webrtc::{SignalingEvent, SignalingStats};
use actr_protocol::{
    AIdCredential, ActrId, Pong, RegisterRequest, RegisterResponse, RouteCandidatesRequest,
    RouteCandidatesResponse, ServiceAvailabilityState, SignalingEnvelope, UnregisterResponse,
};
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use tokio::sync::{Notify, Semaphore, broadcast, mpsc, watch};

struct ForceReconnectFakeSignalingClient {
    connected: AtomicBool,
    connect_once_should_fail: bool,
    disconnect_calls: AtomicUsize,
    connect_once_calls: AtomicUsize,
    auto_reconnect_suppressed: AtomicBool,
    invalidate_generation_calls: AtomicUsize,
    suppress_auto_reconnect_calls: AtomicUsize,
    schedule_auto_reconnect_calls: AtomicUsize,
    schedule_auto_reconnect_reset_backoff_calls: AtomicUsize,
    event_tx: broadcast::Sender<SignalingEvent>,
    /// When set, `disconnect` blocks after signaling `disconnect_entered` and
    /// until `disconnect_release` is notified, so a test can cancel an effect
    /// deterministically while it is mid-teardown.
    block_disconnect: AtomicBool,
    disconnect_entered: Arc<tokio::sync::Notify>,
    disconnect_release: Arc<tokio::sync::Notify>,
}

impl ForceReconnectFakeSignalingClient {
    fn new(connect_once_should_fail: bool) -> Self {
        let (event_tx, _rx) = broadcast::channel(8);
        Self {
            connected: AtomicBool::new(false),
            connect_once_should_fail,
            disconnect_calls: AtomicUsize::new(0),
            connect_once_calls: AtomicUsize::new(0),
            auto_reconnect_suppressed: AtomicBool::new(false),
            invalidate_generation_calls: AtomicUsize::new(0),
            suppress_auto_reconnect_calls: AtomicUsize::new(0),
            schedule_auto_reconnect_calls: AtomicUsize::new(0),
            schedule_auto_reconnect_reset_backoff_calls: AtomicUsize::new(0),
            event_tx,
            block_disconnect: AtomicBool::new(false),
            disconnect_entered: Arc::new(tokio::sync::Notify::new()),
            disconnect_release: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

#[async_trait::async_trait]
impl SignalingClient for ForceReconnectFakeSignalingClient {
    async fn connect(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn connect_once(&self) -> NetworkResult<()> {
        self.connect_once_calls.fetch_add(1, AtomicOrdering::SeqCst);
        if self.connect_once_should_fail {
            return Err(NetworkError::ConnectionError(
                "forced connect_once failure".to_string(),
            ));
        }

        self.connected.store(true, AtomicOrdering::SeqCst);
        Ok(())
    }

    fn suppress_auto_reconnect(&self) {
        self.auto_reconnect_suppressed
            .store(true, AtomicOrdering::SeqCst);
        self.suppress_auto_reconnect_calls
            .fetch_add(1, AtomicOrdering::SeqCst);
    }

    fn invalidate_generation(&self) {
        self.auto_reconnect_suppressed
            .store(true, AtomicOrdering::SeqCst);
        self.invalidate_generation_calls
            .fetch_add(1, AtomicOrdering::SeqCst);
    }

    fn schedule_auto_reconnect(&self) {
        self.auto_reconnect_suppressed
            .store(false, AtomicOrdering::SeqCst);
        self.schedule_auto_reconnect_calls
            .fetch_add(1, AtomicOrdering::SeqCst);
    }

    fn schedule_auto_reconnect_reset_backoff(&self) {
        self.schedule_auto_reconnect_reset_backoff_calls
            .fetch_add(1, AtomicOrdering::SeqCst);
        self.schedule_auto_reconnect();
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        if self.block_disconnect.load(AtomicOrdering::SeqCst) {
            self.disconnect_entered.notify_one();
            self.disconnect_release.notified().await;
        }
        self.disconnect_calls.fetch_add(1, AtomicOrdering::SeqCst);
        self.suppress_auto_reconnect();
        self.connected.store(false, AtomicOrdering::SeqCst);
        Ok(())
    }

    async fn send_register_request(
        &self,
        _request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn send_unregister_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn send_heartbeat(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _availability: ServiceAvailabilityState,
        _power_reserve: f32,
        _mailbox_backlog: f32,
    ) -> NetworkResult<Pong> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn send_route_candidates_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn get_signing_key(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _key_id: u32,
    ) -> NetworkResult<(u32, Vec<u8>)> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    fn is_connected(&self) -> bool {
        self.connected.load(AtomicOrdering::SeqCst)
    }

    fn get_stats(&self) -> SignalingStats {
        SignalingStats::default()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
        self.event_tx.subscribe()
    }

    async fn set_actor_id(&self, _actor_id: ActrId) {}

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}
}

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
fn restarted_monitor_gets_a_fresh_epoch_while_clones_keep_the_incarnation() {
    let (event_tx, _event_rx) = mpsc::channel(1);
    let first = NetworkEventHandle::new(event_tx);
    let shared = first.clone();
    let restarted = first.new_monitor_incarnation();

    assert_eq!(shared.source_epoch, first.source_epoch);
    assert!(restarted.source_epoch > first.source_epoch);
}

#[test]
fn recovery_timer_ids_keep_their_normative_categories() {
    use crate::timer::TimerCategory;

    let cases = [
        (
            tp::TimerId::OfflineCandidate,
            TimerCategory::BusinessHysteresis,
        ),
        (tp::TimerId::ShutdownOverall, TimerCategory::FailureDeadline),
        (
            tp::TimerId::TeardownOverall(tp::TeardownDomain::Cleanup),
            TimerCategory::FailureDeadline,
        ),
        (
            tp::TimerId::TeardownOverall(tp::TeardownDomain::OfflineDisconnect),
            TimerCategory::FailureDeadline,
        ),
        (tp::TimerId::BootstrapPhase, TimerCategory::FailureDeadline),
        (
            tp::TimerId::FailureBackoff(tp::RetryDomain::Recovery),
            TimerCategory::FailureBackoff,
        ),
        (
            tp::TimerId::FailureBackoff(tp::RetryDomain::Cleanup),
            TimerCategory::FailureBackoff,
        ),
        (
            tp::TimerId::FailureBackoff(tp::RetryDomain::Offline),
            TimerCategory::FailureBackoff,
        ),
    ];

    for (id, expected_category) in cases {
        assert_eq!(recovery_timer_definition(id).category, expected_category);
    }
}

#[test]
fn recovery_effect_errors_preserve_typed_network_diagnoses() {
    let cases = [
        (
            NetworkRecoveryError::from_network_error(
                "connect",
                NetworkError::ConnectionError("offline".to_string()),
            ),
            EffectDiagnosis::PathUnreachable {
                stage: "connect: Connection error: offline".to_string(),
            },
        ),
        (
            NetworkRecoveryError::from_network_error(
                "probe",
                NetworkError::TimeoutError("deadline".to_string()),
            ),
            EffectDiagnosis::Timeout {
                stage: "probe: Timeout error: deadline".to_string(),
            },
        ),
        (
            NetworkRecoveryError::from_network_error(
                "connect",
                NetworkError::ResourceExhaustedError("sockets".to_string()),
            ),
            EffectDiagnosis::ResourceExhausted {
                resource: "connect: Resource exhausted: sockets".to_string(),
            },
        ),
        (
            NetworkRecoveryError::from_network_error(
                "connect",
                NetworkError::AuthenticationError("revoked".to_string()),
            ),
            EffectDiagnosis::AuthRejected {
                kind: "connect: Authentication error: revoked".to_string(),
            },
        ),
        (
            NetworkRecoveryError::from_network_error(
                "connect",
                NetworkError::ConfigurationError("endpoint".to_string()),
            ),
            EffectDiagnosis::ConfigRejected {
                detail: "connect: Configuration error: endpoint".to_string(),
            },
        ),
        (
            NetworkRecoveryError::from_network_error(
                "decode",
                NetworkError::DeserializationError("invalid frame".to_string()),
            ),
            EffectDiagnosis::InvariantViolation {
                detail: "decode: Deserialization error: invalid frame".to_string(),
            },
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.into_diagnosis(), expected);
    }
}

#[test]
fn lifecycle_barrier_is_scoped_to_events_that_change_connections() {
    let cases = [
        (
            NetworkEvent::NetworkPathChanged {
                snapshot: snapshot(1, NetworkAvailability::Unavailable),
            },
            true,
        ),
        (
            NetworkEvent::NetworkPathChanged {
                snapshot: snapshot(2, NetworkAvailability::Available),
            },
            true,
        ),
        (
            NetworkEvent::NetworkPathChanged {
                snapshot: snapshot(3, NetworkAvailability::Unknown),
            },
            false,
        ),
        (
            NetworkEvent::AppLifecycleChanged {
                state: AppLifecycleState::Background,
            },
            false,
        ),
        (
            NetworkEvent::AppLifecycleChanged {
                state: AppLifecycleState::Foreground {
                    background_duration_ms: LONG_BACKGROUND_RECONNECT_THRESHOLD_MS - 1,
                },
            },
            false,
        ),
        (
            NetworkEvent::AppLifecycleChanged {
                state: AppLifecycleState::Foreground {
                    background_duration_ms: LONG_BACKGROUND_RECONNECT_THRESHOLD_MS,
                },
            },
            true,
        ),
        (
            NetworkEvent::CleanupConnections {
                reason: CleanupReason::ManualReset,
            },
            true,
        ),
        (
            NetworkEvent::ForceReconnect {
                reason: ReconnectReason::ManualReconnect,
            },
            true,
        ),
    ];

    for (event, expected) in cases {
        assert_eq!(
            network_event_needs_lifecycle_barrier(&event),
            expected,
            "{event:?}"
        );
    }
}

#[test]
fn signaling_directive_hooks_control_auto_reconnect() {
    // Auto-reconnect suppression/resumption is now derived by policy translation
    // as a `SignalingDirective` and executed by the reconciler through these
    // processor hooks (background entry and long-background rebuilds emit
    // `SuppressAutoReconnect`; a short-background probe emits
    // `ResumeAutoReconnect`). This test covers the processor's lowering of those
    // directives onto the signaling client; `translate_tests` covers the
    // derivation itself.
    let signaling = Arc::new(ForceReconnectFakeSignalingClient::new(false));
    let processor = DefaultNetworkEventProcessor::new(signaling.clone(), None);

    processor.invalidate_signaling_connection_attempts();
    assert_eq!(
        signaling
            .invalidate_generation_calls
            .load(AtomicOrdering::SeqCst),
        1,
        "InvalidateConnectionAttempts must fence explicit and automatic attempts"
    );
    assert!(
        signaling
            .auto_reconnect_suppressed
            .load(AtomicOrdering::SeqCst),
        "the generation fence must leave automatic reconnect paused"
    );

    processor.suppress_auto_reconnect();
    assert_eq!(
        signaling
            .suppress_auto_reconnect_calls
            .load(AtomicOrdering::SeqCst),
        1,
        "SuppressAutoReconnect must pause reconnect attempts without disconnecting a healthy socket"
    );
    assert!(
        signaling
            .auto_reconnect_suppressed
            .load(AtomicOrdering::SeqCst),
        "SuppressAutoReconnect must leave automatic reconnect paused"
    );

    processor.resume_auto_reconnect();
    assert!(
        !signaling
            .auto_reconnect_suppressed
            .load(AtomicOrdering::SeqCst),
        "ResumeAutoReconnect must re-enable future automatic reconnects"
    );
}

struct PreemptionFenceProcessor {
    token: CancellationToken,
    fence_calls: AtomicUsize,
    fence_observed_before_cancel: AtomicBool,
}

#[async_trait::async_trait]
impl NetworkEventProcessor for PreemptionFenceProcessor {
    fn invalidate_signaling_connection_attempts(&self) {
        self.fence_observed_before_cancel
            .store(!self.token.is_cancelled(), AtomicOrdering::SeqCst);
        self.fence_calls.fetch_add(1, AtomicOrdering::SeqCst);
    }

    async fn process_network_available(&self) -> Result<(), String> {
        Ok(())
    }

    async fn process_network_lost(&self) -> Result<(), String> {
        Ok(())
    }

    async fn process_network_type_changed(
        &self,
        _is_wifi: bool,
        _is_cellular: bool,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn cleanup_connections(&self) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn stronger_reconnect_fences_restore_before_requesting_cancellation() {
    let mut supervisor = RecoverySupervisor::new(tp::LifecycleProfile::Ungated);
    supervisor.accept(
        tp::Input::AppEnteredForeground {
            observed_background_duration: None,
        },
        Duration::ZERO,
    );
    supervisor.accept(
        tp::Input::SignalingGenerationCommitted {
            generation: 7,
            origin: tp::SignalingOrigin::External,
        },
        Duration::from_millis(1),
    );
    supervisor.accept(tp::Input::AppEnteredBackground, Duration::from_millis(2));
    supervisor.accept(
        tp::Input::SignalingGenerationLost {
            generation: 7,
            cause: tp::SignalingLostCause::RemoteReset,
        },
        Duration::from_millis(3),
    );
    let started = supervisor
        .maybe_start_effect(Duration::from_millis(3))
        .expect("signaling loss should start Restore");
    assert_eq!(
        started.kind,
        crate::lifecycle::recovery_policy::diagnosis::EffectKind::Restore
    );

    let token = CancellationToken::new();
    let recorder = Arc::new(PreemptionFenceProcessor {
        token: token.clone(),
        fence_calls: AtomicUsize::new(0),
        fence_observed_before_cancel: AtomicBool::new(false),
    });
    let processor: Arc<dyn NetworkEventProcessor> = recorder.clone();
    let mut effect = Some(RunningEffect {
        action_id: started.action_id,
        token: token.clone(),
    });
    let (internal_tx, _internal_rx) = mpsc::unbounded_channel();
    let (status_tx, _status_rx) = watch::channel(SupervisorStatus::default());
    let mut timers = HashMap::new();
    let mut last_action_id = Some(started.action_id);
    let mut last_outcome = None;

    let terminate = handle_supervisor_input(
        &mut supervisor,
        tp::Input::AppEnteredForeground {
            observed_background_duration: Some(Duration::from_secs(65)),
        },
        Duration::from_secs(65),
        tokio::time::Instant::now(),
        &internal_tx,
        &mut timers,
        &mut effect,
        &processor,
        &status_tx,
        &mut last_action_id,
        &mut last_outcome,
    );

    assert!(!terminate);
    assert_eq!(recorder.fence_calls.load(AtomicOrdering::SeqCst), 1);
    assert!(
        recorder
            .fence_observed_before_cancel
            .load(AtomicOrdering::SeqCst),
        "the old Restore must lose commit rights before cancellation is requested"
    );
    assert!(token.is_cancelled(), "the old Restore must be cancelled");
}

#[tokio::test]
async fn cancelled_recovery_action_does_not_poison_later_actions() {
    // Regression for the processor-side execution tracker that poisoned itself
    // when the supervisor cancelled an effect after `begin` but before
    // `complete` (a dropped future never runs `complete`). The tracker was
    // removed; the supervisor's `view.execution` is the sole single-flight
    // owner, so a cancelled action can no longer wedge later ones.
    let signaling = Arc::new(ForceReconnectFakeSignalingClient::new(false));
    signaling
        .block_disconnect
        .store(true, AtomicOrdering::SeqCst);
    let processor: Arc<dyn NetworkEventProcessor> =
        Arc::new(DefaultNetworkEventProcessor::new(signaling.clone(), None));

    // Run a ForceReconnect effect and cancel it once it is mid-teardown (blocked
    // in disconnect) but before it can complete — exactly what the reconciler
    // does when a stronger policy preempts a running effect.
    let entered = signaling.disconnect_entered.clone();
    let token = tokio_util::sync::CancellationToken::new();
    let task_processor = processor.clone();
    let task_token = token.clone();
    let task = tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = task_token.cancelled() => {}
            _ = task_processor
                .process_network_recovery_action(NetworkRecoveryAction::ForceReconnect) => {}
        }
    });
    entered.notified().await;
    token.cancel();
    task.await.expect("cancelled effect task joins");

    // A later action still begins and completes normally.
    signaling
        .block_disconnect
        .store(false, AtomicOrdering::SeqCst);
    let result = processor
        .process_network_recovery_action(NetworkRecoveryAction::Restore)
        .await;
    assert!(
        result.is_ok(),
        "a cancelled action must not poison later actions: {result:?}"
    );
}

#[tokio::test]
async fn force_reconnect_reenables_auto_reconnect_after_early_suppression() {
    let signaling = Arc::new(ForceReconnectFakeSignalingClient::new(false));
    let processor = DefaultNetworkEventProcessor::new(signaling.clone(), None);

    // A long-background foreground rebuild suppresses stale auto-reconnect via
    // the `SuppressAutoReconnect` directive the reconciler lowers here.
    processor.suppress_auto_reconnect();
    assert!(
        signaling
            .auto_reconnect_suppressed
            .load(AtomicOrdering::SeqCst),
        "long foreground preparation should pause automatic reconnect"
    );

    processor
        .force_reconnect()
        .await
        .expect("ForceReconnect should restore signaling");

    assert!(
        !signaling
            .auto_reconnect_suppressed
            .load(AtomicOrdering::SeqCst),
        "successful ForceReconnect should re-enable future automatic reconnects"
    );
    assert_eq!(
        signaling
            .schedule_auto_reconnect_reset_backoff_calls
            .load(AtomicOrdering::SeqCst),
        1,
        "ForceReconnect should re-arm automatic reconnect before its explicit restore"
    );
}

#[tokio::test]
async fn force_reconnect_failure_schedules_auto_reconnect() {
    let signaling = Arc::new(ForceReconnectFakeSignalingClient::new(true));
    let processor = DefaultNetworkEventProcessor::new(signaling.clone(), None);

    let result = processor.force_reconnect().await;

    assert!(result.is_err());
    assert_eq!(
        signaling.disconnect_calls.load(AtomicOrdering::SeqCst),
        1,
        "ForceReconnect cleanup should disconnect signaling once"
    );
    assert_eq!(
        signaling.connect_once_calls.load(AtomicOrdering::SeqCst),
        1,
        "ForceReconnect restore should make one quick connect attempt"
    );
    assert_eq!(
        signaling
            .schedule_auto_reconnect_calls
            .load(AtomicOrdering::SeqCst),
        2,
        "ForceReconnect should wake auto-reconnect before restore and keep it scheduled after failure"
    );
    assert_eq!(
        signaling
            .schedule_auto_reconnect_reset_backoff_calls
            .load(AtomicOrdering::SeqCst),
        1,
        "ForceReconnect should reset reconnect backoff before the quick restore attempt"
    );
}

#[tokio::test]
async fn restore_failure_schedules_auto_reconnect_reset_backoff() {
    let signaling = Arc::new(ForceReconnectFakeSignalingClient::new(true));
    let processor = DefaultNetworkEventProcessor::new(signaling.clone(), None);

    let result = processor
        .process_network_recovery_action(NetworkRecoveryAction::Restore)
        .await;

    assert!(result.is_err());
    assert_eq!(
        signaling.connect_once_calls.load(AtomicOrdering::SeqCst),
        1,
        "Restore should make one quick connect attempt"
    );
    assert_eq!(
        signaling
            .schedule_auto_reconnect_reset_backoff_calls
            .load(AtomicOrdering::SeqCst),
        1,
        "failed Restore should reset reconnect backoff"
    );
}

#[tokio::test]
async fn restore_schedules_reset_backoff_before_quick_connect() {
    let signaling = Arc::new(ForceReconnectFakeSignalingClient::new(false));
    let processor = DefaultNetworkEventProcessor::new(signaling.clone(), None);

    processor
        .process_network_recovery_action(NetworkRecoveryAction::Restore)
        .await
        .expect("Restore should connect successfully");

    assert_eq!(
        signaling.connect_once_calls.load(AtomicOrdering::SeqCst),
        1,
        "Restore should make one quick connect attempt"
    );
    assert_eq!(
        signaling
            .schedule_auto_reconnect_reset_backoff_calls
            .load(AtomicOrdering::SeqCst),
        1,
        "Restore should reset reconnect backoff before the quick connect attempt"
    );
}

#[test]
fn snapshot_is_offline_and_should_restore() {
    let offline = snapshot(1, NetworkAvailability::Unavailable);
    assert!(offline.is_offline());
    assert!(!offline.should_restore());

    let online = snapshot(2, NetworkAvailability::Available);
    assert!(!online.is_offline());
    assert!(online.should_restore());

    // Unknown is neither offline (not Unavailable) nor restorable (not Available).
    let unknown = snapshot(3, NetworkAvailability::Unknown);
    assert!(!unknown.is_offline());
    assert!(!unknown.should_restore());
}

// ---------------------------------------------------------------------------
// S3: fact sink plumbing and bounded teardown
// ---------------------------------------------------------------------------

use crate::lifecycle::recovery_policy::diagnosis::EffectOutcome as TpEffectOutcome;
use crate::lifecycle::recovery_policy::translate as tp;

#[test]
fn fact_sink_delivers_facts_to_internal_channel_in_order() {
    let (sink, mut channel) = super::supervisor_internal_channel();
    sink.signaling_generation_committed(7, SignalingFactOrigin::External);
    sink.session_activated(3);
    sink.signaling_generation_lost(7, SignalingFactLostCause::Disconnected);

    let a = channel.rx.try_recv().expect("committed fact");
    assert!(matches!(
        a,
        tp::Input::SignalingGenerationCommitted { generation: 7, .. }
    ));
    let b = channel.rx.try_recv().expect("session fact");
    assert!(matches!(
        b,
        tp::Input::SessionActivated {
            session_generation: 3
        }
    ));
    let c = channel.rx.try_recv().expect("lost fact");
    assert!(matches!(
        c,
        tp::Input::SignalingGenerationLost { generation: 7, .. }
    ));
}

#[test]
fn fact_sink_is_non_blocking_during_a_burst_or_after_close() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let sink = SupervisorFactSink::new(tx);

    sink.session_activated(1);
    sink.session_activated(2);
    assert!(matches!(
        rx.try_recv().expect("first fact should occupy the channel"),
        tp::Input::SessionActivated {
            session_generation: 1
        }
    ));
    assert!(matches!(
        rx.try_recv().expect("second fact should remain queued"),
        tp::Input::SessionActivated {
            session_generation: 2
        }
    ));

    drop(rx);
    sink.session_activated(3);
}

#[test]
fn fact_sink_retains_terminal_generation_fact_after_capacity_burst() {
    let (sink, mut channel) = super::supervisor_internal_channel();

    for generation in 1..=128 {
        sink.session_activated(generation);
    }
    sink.signaling_generation_lost(77, SignalingFactLostCause::RemoteReset);

    let mut received = 0;
    let mut saw_terminal_loss = false;
    while let Ok(input) = channel.rx.try_recv() {
        received += 1;
        saw_terminal_loss |= matches!(
            input,
            tp::Input::SignalingGenerationLost { generation: 77, .. }
        );
    }

    assert_eq!(received, 129, "a fact burst must remain lossless");
    assert!(
        saw_terminal_loss,
        "the terminal generation loss must survive a full fact channel"
    );
}

#[derive(Clone, Copy)]
enum EffectExit {
    Succeed,
    Fail,
    Panic,
    Pending,
}

struct EffectExitProcessor {
    exit: EffectExit,
    entered: Notify,
}

impl EffectExitProcessor {
    fn new(exit: EffectExit) -> Self {
        Self {
            exit,
            entered: Notify::new(),
        }
    }
}

#[async_trait::async_trait]
impl NetworkEventProcessor for EffectExitProcessor {
    async fn process_network_available(&self) -> Result<(), String> {
        Ok(())
    }

    async fn process_network_lost(&self) -> Result<(), String> {
        Ok(())
    }

    async fn process_network_type_changed(
        &self,
        _is_wifi: bool,
        _is_cellular: bool,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn cleanup_connections(&self) -> Result<(), String> {
        Ok(())
    }

    async fn process_network_recovery_action(
        &self,
        _action: NetworkRecoveryAction,
    ) -> Result<(), String> {
        self.entered.notify_one();
        match self.exit {
            EffectExit::Succeed => Ok(()),
            EffectExit::Fail => Err("forced effect failure".to_string()),
            EffectExit::Panic => panic!("forced effect panic"),
            EffectExit::Pending => std::future::pending().await,
        }
    }
}

fn restore_effect(action_id: u64) -> StartedEffect {
    StartedEffect {
        action_id,
        kind: crate::lifecycle::recovery_policy::diagnosis::EffectKind::Restore,
        action: tp::Action::Restore,
        captured_revision: 7,
        teardown_budget: None,
    }
}

async fn receive_effect_completion(
    rx: &mut mpsc::UnboundedReceiver<tp::Input>,
) -> crate::lifecycle::recovery_policy::diagnosis::EffectOutcome {
    let input = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
        .await
        .expect("effect should complete within the test deadline")
        .expect("effect completion channel should stay open");
    let tp::Input::EffectCompleted {
        action_id,
        kind,
        policy_revision,
        outcome,
    } = input
    else {
        panic!("expected EffectCompleted, got {input:?}");
    };
    assert_eq!(action_id, 1);
    assert_eq!(
        kind,
        crate::lifecycle::recovery_policy::diagnosis::EffectKind::Restore
    );
    assert_eq!(policy_revision, 7);
    outcome
}

#[tokio::test]
async fn effect_reports_exactly_one_terminal_completion_for_every_exit_path() {
    let cases = [
        (EffectExit::Succeed, "succeed"),
        (EffectExit::Fail, "fail"),
        (EffectExit::Panic, "panic"),
        (EffectExit::Pending, "cancel"),
    ];

    for (exit, name) in cases {
        let processor = Arc::new(EffectExitProcessor::new(exit));
        let token = CancellationToken::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        spawn_effect(restore_effect(1), token.clone(), processor.clone(), tx);

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            processor.entered.notified(),
        )
        .await
        .unwrap_or_else(|_| panic!("{name} effect should start"));
        if matches!(exit, EffectExit::Pending) {
            token.cancel();
        }

        let outcome = receive_effect_completion(&mut rx).await;
        match exit {
            EffectExit::Succeed => assert!(matches!(outcome, TpEffectOutcome::Succeeded)),
            EffectExit::Fail => assert!(matches!(outcome, TpEffectOutcome::Failed { .. })),
            EffectExit::Panic => assert!(matches!(
                outcome,
                TpEffectOutcome::Aborted {
                    cause: AbortCause::PanicOrContractViolation
                }
            )),
            EffectExit::Pending => assert!(matches!(outcome, TpEffectOutcome::Cancelled)),
        }

        tokio::task::yield_now().await;
        assert!(
            matches!(
                rx.try_recv(),
                Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected)
            ),
            "{name} effect must report exactly one terminal completion"
        );
    }
}

#[tokio::test]
async fn effect_completion_queues_behind_existing_internal_input() {
    let processor = Arc::new(EffectExitProcessor::new(EffectExit::Succeed));
    let (tx, mut rx) = mpsc::unbounded_channel();
    tx.send(tp::Input::ShutdownRequested)
        .expect("test should queue the earlier internal input");

    spawn_effect(
        restore_effect(1),
        CancellationToken::new(),
        processor.clone(),
        tx,
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        processor.entered.notified(),
    )
    .await
    .expect("effect should run while an earlier internal input is queued");
    tokio::task::yield_now().await;

    assert!(matches!(rx.try_recv(), Ok(tp::Input::ShutdownRequested)));
    assert!(matches!(
        receive_effect_completion(&mut rx).await,
        TpEffectOutcome::Succeeded
    ));
}

struct CleanupSessionProcessor {
    actions: StdMutex<Vec<NetworkRecoveryAction>>,
    cleanup_entered: Notify,
    cleanup_release: Notify,
    recovery_entered: Notify,
}

impl CleanupSessionProcessor {
    fn actions(&self) -> Vec<NetworkRecoveryAction> {
        self.actions.lock().expect("actions mutex poisoned").clone()
    }
}

#[async_trait::async_trait]
impl NetworkEventProcessor for CleanupSessionProcessor {
    async fn process_network_available(&self) -> Result<(), String> {
        Ok(())
    }

    async fn process_network_lost(&self) -> Result<(), String> {
        Ok(())
    }

    async fn process_network_type_changed(
        &self,
        _is_wifi: bool,
        _is_cellular: bool,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn cleanup_connections(&self) -> Result<(), String> {
        Ok(())
    }

    async fn process_network_recovery_action(
        &self,
        action: NetworkRecoveryAction,
    ) -> Result<(), String> {
        self.actions
            .lock()
            .expect("actions mutex poisoned")
            .push(action);
        self.recovery_entered.notify_one();
        Ok(())
    }

    async fn run_bounded_teardown(
        &self,
        action: NetworkRecoveryAction,
        _budget: std::time::Duration,
    ) -> TeardownReport {
        self.actions
            .lock()
            .expect("actions mutex poisoned")
            .push(action);
        self.cleanup_entered.notify_one();
        self.cleanup_release.notified().await;
        TeardownReport::succeeded()
    }
}

#[tokio::test]
async fn session_activated_during_cleanup_runs_post_cleanup_recovery() {
    let processor = Arc::new(CleanupSessionProcessor {
        actions: StdMutex::new(Vec::new()),
        cleanup_entered: Notify::new(),
        cleanup_release: Notify::new(),
        recovery_entered: Notify::new(),
    });
    let (event_tx, event_rx) = mpsc::channel(8);
    let (fact_sink, channel) = supervisor_internal_channel();
    let handle = NetworkEventHandle::new_with_fact_sink(event_tx, fact_sink);
    let shutdown = CancellationToken::new();
    let reconciler = tokio::spawn(run_network_event_reconciler_with_channel(
        event_rx,
        channel,
        processor.clone(),
        shutdown.clone(),
    ));

    let result = handle
        .cleanup_connections(CleanupReason::ManualReset)
        .await
        .expect("cleanup should be accepted");
    assert!(result.success);
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        processor.cleanup_entered.notified(),
    )
    .await
    .expect("cleanup effect should start");

    handle.notify_session_activated(2);
    processor.cleanup_release.notify_one();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        processor.recovery_entered.notified(),
    )
    .await
    .expect("new session should derive recovery after cleanup");

    assert_eq!(
        processor.actions(),
        vec![
            NetworkRecoveryAction::CleanupOnly,
            NetworkRecoveryAction::Restore
        ]
    );

    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
}

struct ControlledRecoveryProcessor {
    started_tx: mpsc::UnboundedSender<NetworkRecoveryAction>,
    release: Semaphore,
}

#[async_trait::async_trait]
impl NetworkEventProcessor for ControlledRecoveryProcessor {
    async fn process_network_available(&self) -> Result<(), String> {
        Ok(())
    }

    async fn process_network_lost(&self) -> Result<(), String> {
        Ok(())
    }

    async fn process_network_type_changed(
        &self,
        _is_wifi: bool,
        _is_cellular: bool,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn cleanup_connections(&self) -> Result<(), String> {
        Ok(())
    }

    async fn process_network_recovery_action(
        &self,
        action: NetworkRecoveryAction,
    ) -> Result<(), String> {
        self.started_tx
            .send(action)
            .map_err(|_| "test action receiver closed".to_string())?;
        self.release
            .acquire()
            .await
            .map_err(|_| "test release semaphore closed".to_string())?
            .forget();
        Ok(())
    }
}

fn online_route_snapshot(sequence: u64, wifi: bool) -> NetworkSnapshot {
    NetworkSnapshot {
        sequence,
        availability: NetworkAvailability::Available,
        transport: NetworkTransportFlags {
            wifi,
            cellular: !wifi,
            ..NetworkTransportFlags::default()
        },
        is_expensive: !wifi,
        is_constrained: false,
    }
}

#[tokio::test]
async fn gated_reconciler_waits_for_authoritative_foreground() {
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let processor = Arc::new(ControlledRecoveryProcessor {
        started_tx,
        release: Semaphore::new(0),
    });
    let (event_tx, event_rx) = mpsc::channel(16);
    let (fact_sink, channel) =
        supervisor_internal_channel_with_profile(tp::LifecycleProfile::Gated);
    let handle = NetworkEventHandle::new_with_fact_sink(event_tx, fact_sink);
    let SupervisorInternalChannel {
        tx: internal_tx,
        rx: internal_rx,
        profile,
        clock_origin,
    } = channel;
    let (status_tx, status_rx) = watch::channel(SupervisorStatus::default());
    let shutdown = CancellationToken::new();
    let reconciler = tokio::spawn(reconcile_loop(
        event_rx,
        internal_tx,
        internal_rx,
        processor.clone(),
        shutdown.clone(),
        status_tx,
        ReconcilerConfig {
            profile,
            clock_origin,
        },
    ));

    handle
        .handle_network_path_changed(online_route_snapshot(1, true))
        .await
        .expect("initial online route should be accepted");
    assert_eq!(status_rx.borrow().policy_revision, 1);
    assert_eq!(
        status_rx.borrow().last_action_id,
        None,
        "Gated recovery must stay ineligible while app phase is Unknown"
    );
    assert!(started_rx.try_recv().is_err());

    handle
        .handle_app_lifecycle_changed(AppLifecycleState::Foreground {
            background_duration_ms: 0,
        })
        .await
        .expect("authoritative foreground should be accepted");
    assert_eq!(status_rx.borrow().last_action_id, Some(1));
    assert_eq!(
        started_rx.recv().await,
        Some(NetworkRecoveryAction::Restore)
    );

    processor.release.add_permits(1);
    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
}

#[tokio::test]
async fn current_effect_fact_and_event_storm_preserve_monotonic_supervisor_status() {
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let processor = Arc::new(ControlledRecoveryProcessor {
        started_tx,
        release: Semaphore::new(0),
    });
    let (event_tx, event_rx) = mpsc::channel(16);
    let (fact_sink, channel) = supervisor_internal_channel();
    let handle = NetworkEventHandle::new_with_fact_sink(event_tx, fact_sink.clone());
    let SupervisorInternalChannel {
        tx: internal_tx,
        rx: internal_rx,
        profile,
        clock_origin,
    } = channel;
    let (status_tx, mut status_rx) = watch::channel(SupervisorStatus::default());
    let shutdown = CancellationToken::new();
    let reconciler = tokio::spawn(reconcile_loop(
        event_rx,
        internal_tx,
        internal_rx,
        processor.clone(),
        shutdown.clone(),
        status_tx,
        ReconcilerConfig {
            profile,
            clock_origin,
        },
    ));

    handle
        .handle_network_path_changed(online_route_snapshot(1, true))
        .await
        .expect("initial online route should be accepted");
    assert_eq!(
        started_rx.recv().await,
        Some(NetworkRecoveryAction::Restore)
    );
    let mut statuses = vec![status_rx.borrow_and_update().clone()];
    assert_eq!(statuses[0].policy_revision, 1);
    assert_eq!(statuses[0].last_action_id, Some(1));
    assert_eq!(statuses[0].last_outcome, None);

    fact_sink
        .signaling_generation_committed(7, SignalingFactOrigin::CurrentEffect { action_id: 1 });
    status_rx
        .wait_for(|status| status.policy_revision == 2)
        .await
        .expect("current-effect fact should be reconciled");
    statuses.push(status_rx.borrow_and_update().clone());

    for sequence in 2..=11 {
        handle
            .handle_network_path_changed(online_route_snapshot(sequence, sequence % 2 == 1))
            .await
            .expect("material route change should be accepted");
        let status = status_rx.borrow_and_update().clone();
        assert_eq!(status.policy_revision, sequence + 1);
        assert_eq!(status.last_action_id, Some(1));
        assert_eq!(status.last_outcome, None);
        statuses.push(status);
    }

    for sequence in 12..=111 {
        handle
            .handle_network_path_changed(online_route_snapshot(sequence, true))
            .await
            .expect("duplicate route snapshot should be accepted");
        let status = status_rx.borrow_and_update().clone();
        assert_eq!(
            status.policy_revision, 12,
            "structural duplicates must not advance policy"
        );
        assert_eq!(status.last_action_id, Some(1));
        assert_eq!(status.last_outcome, None);
        statuses.push(status);
    }

    processor.release.add_permits(1);
    status_rx
        .wait_for(|status| status.last_outcome == Some(ObservedOutcome::Succeeded))
        .await
        .expect("the first effect completion should remain current");
    let first_completion = status_rx.borrow_and_update().clone();
    assert_eq!(first_completion.policy_revision, 12);
    assert_eq!(first_completion.last_action_id, Some(1));
    statuses.push(first_completion);

    handle
        .force_reconnect(ReconnectReason::ManualReconnect)
        .await
        .expect("a later recovery request should be accepted");
    assert_eq!(
        started_rx.recv().await,
        Some(NetworkRecoveryAction::ForceReconnect)
    );
    let second_started = status_rx.borrow_and_update().clone();
    assert_eq!(second_started.policy_revision, 13);
    assert_eq!(second_started.last_action_id, Some(2));
    assert_eq!(
        second_started.last_outcome,
        Some(ObservedOutcome::Succeeded),
        "the previous terminal outcome should remain observable while new work runs"
    );
    statuses.push(second_started);

    processor.release.add_permits(1);
    status_rx
        .changed()
        .await
        .expect("the second effect completion should publish status");
    let second_completion = status_rx.borrow_and_update().clone();
    assert_eq!(second_completion.policy_revision, 13);
    assert_eq!(second_completion.last_action_id, Some(2));
    assert_eq!(
        second_completion.last_outcome,
        Some(ObservedOutcome::Succeeded)
    );
    statuses.push(second_completion);

    assert!(statuses.windows(2).all(|pair| {
        pair[0].policy_revision <= pair[1].policy_revision
            && pair[0].last_action_id <= pair[1].last_action_id
    }));

    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
}

#[test]
fn teardown_outcome_maps_every_report_class() {
    assert!(matches!(
        super::teardown_outcome(TeardownReport::succeeded()),
        TpEffectOutcome::Succeeded
    ));
    assert!(matches!(
        super::teardown_outcome(TeardownReport::completed_with_residuals(vec!["x".into()])),
        TpEffectOutcome::CompletedWithResiduals { .. }
    ));
    assert!(matches!(
        super::teardown_outcome(TeardownReport::abandoned(vec!["x".into()])),
        TpEffectOutcome::Abandoned { .. }
    ));
    let failed = TeardownReport {
        reached_goal: false,
        deadline_reached: false,
        residuals: vec![],
    };
    assert!(matches!(
        super::teardown_outcome(failed),
        TpEffectOutcome::Failed { .. }
    ));
}

#[derive(Clone, Copy)]
enum DisconnectMode {
    Err,
    Hang,
}

struct TeardownFakeSignaling {
    connected: AtomicBool,
    disconnect_mode: DisconnectMode,
    invalidated: AtomicBool,
    hang: tokio::sync::Notify,
    event_tx: broadcast::Sender<SignalingEvent>,
}

impl TeardownFakeSignaling {
    fn new(mode: DisconnectMode) -> Self {
        let (event_tx, _rx) = broadcast::channel(8);
        Self {
            connected: AtomicBool::new(true),
            disconnect_mode: mode,
            invalidated: AtomicBool::new(false),
            hang: tokio::sync::Notify::new(),
            event_tx,
        }
    }
}

#[async_trait::async_trait]
impl SignalingClient for TeardownFakeSignaling {
    async fn connect(&self) -> NetworkResult<()> {
        Ok(())
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        match self.disconnect_mode {
            DisconnectMode::Err => Err(NetworkError::ConnectionError(
                "forced disconnect failure".to_string(),
            )),
            DisconnectMode::Hang => {
                // Never notified: the caller's overall budget must abandon this.
                self.hang.notified().await;
                Ok(())
            }
        }
    }

    fn invalidate_generation(&self) {
        self.invalidated.store(true, AtomicOrdering::SeqCst);
    }

    async fn send_register_request(
        &self,
        _request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn send_unregister_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn send_heartbeat(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _availability: ServiceAvailabilityState,
        _power_reserve: f32,
        _mailbox_backlog: f32,
    ) -> NetworkResult<Pong> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn send_route_candidates_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn get_signing_key(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _key_id: u32,
    ) -> NetworkResult<(u32, Vec<u8>)> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        Err(NetworkError::ConnectionError("unused".to_string()))
    }

    fn is_connected(&self) -> bool {
        self.connected.load(AtomicOrdering::SeqCst)
    }

    fn get_stats(&self) -> SignalingStats {
        SignalingStats::default()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
        self.event_tx.subscribe()
    }

    async fn set_actor_id(&self, _actor_id: ActrId) {}

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}
}

#[tokio::test]
async fn bounded_cleanup_invalidates_up_front_and_records_residuals() {
    let signaling = Arc::new(TeardownFakeSignaling::new(DisconnectMode::Err));
    let processor = DefaultNetworkEventProcessor::new(signaling.clone(), None);

    let report = processor
        .bounded_cleanup(std::time::Duration::from_secs(10))
        .await;

    assert!(
        signaling.invalidated.load(AtomicOrdering::SeqCst),
        "commit rights are invalidated synchronously before any physical step"
    );
    assert!(report.reached_goal, "local teardown completed");
    assert!(!report.deadline_reached);
    assert_eq!(
        report.residuals.len(),
        1,
        "the failed disconnect step is recorded as a residual, not an early return"
    );
}

#[tokio::test(start_paused = true)]
async fn bounded_cleanup_abandons_remaining_steps_at_budget() {
    let signaling = Arc::new(TeardownFakeSignaling::new(DisconnectMode::Hang));
    let processor = DefaultNetworkEventProcessor::new(signaling.clone(), None);
    let budget = std::time::Duration::from_millis(100);

    // Drive time forward past the budget while the hanging disconnect is in
    // flight; the overall deadline must abandon the remaining steps.
    let (report, ()) = tokio::join!(processor.bounded_cleanup(budget), async {
        tokio::time::advance(std::time::Duration::from_millis(150)).await;
    });

    assert!(signaling.invalidated.load(AtomicOrdering::SeqCst));
    assert!(report.deadline_reached, "budget expired");
    assert!(!report.reached_goal, "goal not confirmed");
    assert_eq!(report.residuals.len(), 1);
}
