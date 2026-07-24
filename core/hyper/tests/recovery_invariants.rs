//! RFC-0400 required-invariant traceability matrix and black-box confirmation
//! suite for the responsive connection-recovery supervisor.
//!
//! This file exercises the supervisor end to end through its public surface
//! (`NetworkEventHandle`, `run_network_event_reconciler[_with_status]`,
//! `SupervisorStatus`) instead of the crate-private `translate()` reducer or
//! `RecoverySupervisor` engine. It complements, rather than replaces, the
//! deep synchronous unit coverage in `src/lifecycle/recovery_supervisor_tests.rs`
//! and `src/lifecycle/recovery_policy/{translate_tests,classification}.rs`:
//! those prove the pure policy decisions; this proves the async shell (timers,
//! effect spawning, acceptance, cancellation) actually wires them up correctly.
//!
//! Every test name embeds the invariant number it demonstrates (`invN_...`).
//! Timer-driven behavior uses `#[tokio::test(start_paused = true)]` plus
//! `tokio::time::advance` / the paused clock's auto-advance; cross-task
//! coordination waits on a real notification (the signaling client's
//! broadcast event stream, or `SupervisorStatus`'s watch channel) — never a
//! wall-clock sleep.
//!
//! ## Invariant -> test mapping (RFC-0400 "Required invariants")
//!
//! Phase 2 (this round's primary scope: invariants 1-11, 25-32; each has at
//! least one deterministic test, several have both a pure-engine test and a
//! black-box confirmation here):
//!
//! | # | Invariant (short) | Test(s) |
//! |---|---|---|
//! | 1 | Stale/duplicate snapshot inert; new epoch resets sequence | here: `inv1_stale_and_duplicate_snapshots_cannot_advance_policy_but_a_new_epoch_does`; unit: `recovery_supervisor_tests::inv1_duplicate_and_stale_snapshots_cannot_move_path_state`; reducer: `translate_tests::{snapshot_stale_epoch_or_sequence_is_discarded, snapshot_structural_duplicate_is_discarded}` |
//! | 2 | Fast rollback: no disconnect, supersedes/cancels pending or running disconnect, derives restoration | here: `inv2_online_rollback_before_grace_performs_no_disconnect`; unit: `recovery_supervisor_tests::{inv2_online_before_grace_rolls_back_without_disconnect, inv2_online_cancels_a_running_confirmed_disconnect_and_derives_restoration}`; reducer: `translate_tests::snapshot_online_supersedes_pending_disconnect` |
//! | 3 | `OfflineCandidate` permits existing traffic, no new negotiation, no teardown barrier | here: `inv3_offline_candidate_permits_existing_session_without_immediate_disconnect`; unit: `recovery_supervisor_tests::inv3_offline_candidate_arms_grace_and_permits_existing_traffic` (asserts `SendProjection::ExistingOnly` directly, not observable through the public API) |
//! | 4 | Cleanup cannot acknowledge a later recovery fact | unit: `recovery_supervisor_tests::{inv4_cleanup_supersedes_pending_recovery_by_revision, inv4_cleanup_completion_cannot_acknowledge_a_later_recovery_fact}`; integration: `network_event_debounce::test_l1_cleanup_is_a_batch_barrier_for_later_recovery_facts` |
//! | 5 | `LoggedOut`/`Terminating` not reactivated by network facts | unit: `recovery_supervisor_tests::{inv5_logout_gates_recovery_and_is_not_reactivated_by_network_facts, inv30_app_terminating_enters_terminating_and_blocks_sends}` (the latter also asserts the `Terminating` half) |
//! | 6 | Duplicate foreground creates no recovery work | here: `inv6_duplicate_foreground_creates_no_recovery_work`; unit: `recovery_supervisor_tests::inv6_cold_and_duplicate_foreground_create_no_recovery_work` |
//! | 7 | Background gates new active recovery only under `Gated`; preserves intent/healthy sessions | unit (only place `Gated` is constructible): `recovery_supervisor_tests::{inv7_gated_profile_denies_eligibility_until_foreground, inv7_gated_bootstrap_expiry_keeps_profile_gated, inv7_gated_profile_background_gates_recovery_but_preserves_intent}`; integration documents the `Ungated` compatibility side (background never gates): `network_event_debounce::test_background_preserves_healthy_session_and_admits_reconnect_under_ungated` |
//! | 8 | Lifecycle recovery effects single-flight while responsive; independent resource-scoped flights not needlessly serialized | unit: `recovery_supervisor_tests::inv8_execution_is_single_flight_while_supervisor_stays_responsive`; the resource-scoped independence half is an architectural property of per-destination flights in `transport::peer_transport` (each destination owns its own singleflight state, not a shared lock) exercised by `peer_transport_tests::{inv13_cancelled_creator_releases_ownership_without_erasing_replacement, inv14_only_current_destination_flight_can_commit_transport}` |
//! | 9 | Stale/mismatched completion cannot acknowledge; weaker effect cannot acknowledge stronger intent | unit: `recovery_supervisor_tests::{inv9_stale_completion_cannot_acknowledge_work, inv9_weaker_effect_completion_cannot_acknowledge_stronger_intent}` |
//! | 10 | Failed work cannot hot-loop; availability never parks; `Parked` only from precondition/pause with non-empty mask | here: `inv10_repeated_availability_failures_back_off_instead_of_hot_looping`; unit: `recovery_supervisor_tests::{inv10_availability_failure_backs_off_and_only_matching_deadline_rearms, inv10_auth_rejection_parks_recovery_until_a_clearing_trigger}` |
//! | 11 | Acceptance decoupled from effect completion; caller timeout cannot orphan work | here: `inv11_acceptance_does_not_wait_for_effect_completion`; integration: `network_event_debounce::{test_l1_reconciler_shutdown_during_offline_grace_is_bounded, test_network_event_handle_pending_request_is_bounded_by_deadline}` |
//! | 25 | No effect classifies its own failure; verdict from the per-kind classification table alone | unit: `recovery_supervisor_tests::inv25_classification_table_alone_determines_the_verdict`; structurally enforced by `EffectOutcome::Failed` carrying only a typed `diagnosis` field (`recovery_policy::diagnosis`), never a verdict |
//! | 26 | Translation deterministic; reducer covers every normative row | **S1, confirmed, not rewritten**: `recovery_policy::translate_tests::translation_is_byte_stable_for_identical_arguments`, plus full-table coverage across the file's 144 tests and `recovery_policy::classification`'s 23 tests |
//! | 27 | Every non-`Idle` execution gets exactly one accepted terminal completion; only the supervisor mutates it | unit: `recovery_supervisor_tests::inv27_cleanup_preempts_a_running_probe`; "only the supervisor mutates it" is a structural property (`RecoverySupervisor::view().execution` is private, mutated only by `apply_execution` in the same module) |
//! | 28 | Obligations end only through enumerated extinguishment paths; fact reversal extinguishes only its own domain | unit: `recovery_supervisor_tests::inv28_fact_reversal_extinguishes_only_its_own_domain` |
//! | 29 | `DisconnectPending` implies `NetworkPath == Offline`; fall-through logged, never silent | unit: `recovery_supervisor_tests::inv29_offline_grace_expiry_commits_and_derives_confirmed_disconnect`; the logging half is a code-review property of the translation table's `_ =>` arms (each logs `tracing::error!`), not independently unit-tested |
//! | 30 | Teardown/shutdown terminate within their overall deadline; only unbounded state is `InvariantViolation` park outside `Terminating` | unit: `recovery_supervisor_tests::inv30_app_terminating_enters_terminating_and_blocks_sends`; reducer: `translate_tests::{teardown_deadline_abandons_backing_off_obligation, teardown_deadline_ignored_while_effect_running, shutdown_deadline_aborts_when_terminating_and_matching}`, `classification::teardown_invariant_violation_parks_or_abandons_by_mode` |
//! | 31 | `SessionActivated` during pending cleanup derives a post-cleanup obligation; cleanup's own completion cannot extinguish it | unit: `recovery_supervisor_tests::inv31_session_activated_during_cleanup_derives_a_post_cleanup_obligation` |
//! | 32 | A current-effect-origin fact cannot make that effect's own completion stale | reducer (S1, confirmed, not rewritten): `translate_tests::signaling_committed_current_effect_is_covered_output` |
//!
//! Phase 3 (invariants 12-16, generation/cancellation safety):
//!
//! | # | Invariant (short) | Representative existing test(s) |
//! |---|---|---|
//! | 12 | Stale signaling generation cannot publish `Connected` | `wire::webrtc::signaling::tests::inv12_stale_signaling_generation_cannot_publish_connected` |
//! | 13 | Cancelled creator releases ownership without erasing its replacement | `transport::peer_transport_tests::inv13_cancelled_creator_releases_ownership_without_erasing_replacement` |
//! | 14 | Only the current destination flight may commit transport state | `transport::peer_transport_tests::inv14_only_current_destination_flight_can_commit_transport` |
//! | 15 | Close and late connection success/failure linearized | `wire::webrtc::coordinator::tests::{inv15_close_all_rejects_late_successful_peer_publication, inv15_failed_close_all_commit_does_not_partially_remove_peer}` |
//! | 16 | Transport creation racing per-peer/close-all teardown cannot deadlock | `wire::webrtc::coordinator::tests::{inv16_close_all_rejects_racing_peer_creation, inv16_reentrant_close_all_does_not_deadlock}` |
//!
//! Phase 4 (invariants 17-24, event-driven production paths; each has a
//! deterministic dedicated test, complementing the implementation landed by
//! `perf(hyper): replace transport polling with notifications`):
//!
//! | # | Invariant (short) | Representative existing test(s) |
//! |---|---|---|
//! | 17 | Every DataChannel waiter wakes on Open/Closed | `transport::lane::tests::inv17_data_channel_state_change_wakes_every_waiter` |
//! | 18 | Initial readiness/ICE gathering cannot lose a transition | `wire::webrtc::coordinator::tests::{inv18_ice_gathering_transition_is_retained_after_listener_is_armed, inv18_initial_readiness_subscribes_before_accepting_transition, inv18_initial_connecting_state_emits_connecting_hook, inv18_initial_failure_emits_idle_not_recovering, inv18_connecting_state_reopens_connected_hook_window}` |
//! | 19 | Duplicate peer-state events don't extend stale-peer lifetime or combine a new state with an old timestamp | `wire::webrtc::coordinator::tests::{inv19_duplicate_peer_state_does_not_extend_stale_reap_deadline, inv19_stale_peer_reap_uses_dedicated_disconnected_threshold, inv19_new_peer_state_cannot_reuse_the_previous_states_timestamp}` |
//! | 20 | Empty mailbox storage doesn't starve in-flight reply/ack | `lifecycle::node::tests::inv20_empty_mailbox_keeps_driving_inflight_reply_and_ack_tails` |
//! | 21 | Available quota can't stay idle from collapsed release notifications | `wasm::runtime_limits::tests::inv21_quota_release_generation_survives_collapsed_notifications` |
//! | 22 | Successful transitions don't wait on fixed polling or consume a failure deadline | `wire::webrtc::coordinator::tests::inv22_success_transition_completes_without_polling_or_failure_deadline`; `wire::webrtc::signaling_tests::inv22_reconnect_manager_lifetime_uses_drop_signal_not_periodic_polling` |
//! | 23 | Parallel shutdown bounded by one overall deadline, not per-child | `wire::webrtc::coordinator::tests::{inv23_close_all_hooks_share_one_overall_deadline, inv23_coordinator_background_tasks_are_joined_by_shutdown}` |
//! | 24 | Every timer uses the audited facade with one inventory classification | `timer::tests::{inv24_inventory_metadata_is_complete_and_unique, inv24_production_timer_calls_and_inventory_do_not_drift}` plus the explicit RFC-0400 timer-inventory CI gate |

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use actr_hyper::lifecycle::{
    AppLifecycleState, CleanupReason, CredentialState, DefaultNetworkEventProcessor,
    NetworkAvailability, NetworkEvent, NetworkEventHandle, NetworkEventProcessor,
    NetworkEventRequest, NetworkSnapshot, NetworkTransportFlags, SupervisorStatus,
    run_network_event_reconciler, run_network_event_reconciler_with_status,
};
use actr_hyper::transport::{NetworkError, NetworkResult};
use actr_hyper::wire::webrtc::{DisconnectReason, SignalingClient, SignalingEvent, SignalingStats};
use actr_protocol::{
    AIdCredential, ActrId, Pong, RegisterRequest, RegisterResponse, RouteCandidatesRequest,
    RouteCandidatesResponse, SignalingEnvelope, UnregisterResponse,
};
use tokio::sync::{broadcast, watch};

/// A minimal, controllable [`SignalingClient`] fake. Trimmed from the fixture
/// in `network_event_debounce.rs` to only what these invariant tests drive.
struct FakeSignalingClient {
    connected: AtomicBool,
    connections: AtomicU64,
    disconnections: AtomicU64,
    probe_calls: AtomicU64,
    probe_success: AtomicBool,
    connect_should_fail: AtomicBool,
    connect_once_calls: AtomicU64,
    /// Every attempt at reaching signaling — probe, `connect`, or
    /// `connect_once` — regardless of which one the processor happens to pick
    /// once the client has fallen disconnected. This is the right observable
    /// for "did recovery retry", since a failed reconnect makes later cycles
    /// skip straight past `probe_alive` (which only runs while connected).
    recovery_attempts: AtomicU64,
    event_tx: broadcast::Sender<SignalingEvent>,
}

impl FakeSignalingClient {
    fn new() -> Self {
        let (event_tx, _rx) = broadcast::channel(64);
        Self {
            connected: AtomicBool::new(false),
            connections: AtomicU64::new(0),
            disconnections: AtomicU64::new(0),
            probe_calls: AtomicU64::new(0),
            probe_success: AtomicBool::new(true),
            connect_should_fail: AtomicBool::new(false),
            connect_once_calls: AtomicU64::new(0),
            recovery_attempts: AtomicU64::new(0),
            event_tx,
        }
    }

    fn stats(&self) -> SignalingStats {
        SignalingStats {
            connections: self.connections.load(Ordering::SeqCst),
            disconnections: self.disconnections.load(Ordering::SeqCst),
            ..SignalingStats::default()
        }
    }

    fn probe_calls(&self) -> u64 {
        self.probe_calls.load(Ordering::SeqCst)
    }

    fn recovery_attempts(&self) -> u64 {
        self.recovery_attempts.load(Ordering::SeqCst)
    }

    /// After this, every `probe_alive` call fails (an availability-family
    /// diagnosis at the policy layer).
    fn set_probe_success(&self, success: bool) {
        self.probe_success.store(success, Ordering::SeqCst);
    }

    /// After this, `connect`/`connect_once` also fail, so a failed probe
    /// cannot be papered over by a same-cycle reconnect: the whole recovery
    /// effect genuinely fails and reaches the classification table.
    fn set_connect_should_fail(&self, fail: bool) {
        self.connect_should_fail.store(fail, Ordering::SeqCst);
    }

    fn publish_connected(&self) {
        self.connected.store(true, Ordering::SeqCst);
        self.connections.fetch_add(1, Ordering::SeqCst);
        let _ = self.event_tx.send(SignalingEvent::Connected);
    }
}

#[async_trait::async_trait]
impl SignalingClient for FakeSignalingClient {
    async fn connect(&self) -> NetworkResult<()> {
        self.recovery_attempts.fetch_add(1, Ordering::SeqCst);
        if self.connect_should_fail.load(Ordering::SeqCst) {
            return Err(NetworkError::ConnectionError(
                "fake signaling refuses to connect".to_string(),
            ));
        }
        self.publish_connected();
        Ok(())
    }

    async fn connect_once(&self) -> NetworkResult<()> {
        self.connect_once_calls.fetch_add(1, Ordering::SeqCst);
        self.recovery_attempts.fetch_add(1, Ordering::SeqCst);
        if self.connect_should_fail.load(Ordering::SeqCst) {
            return Err(NetworkError::ConnectionError(
                "fake signaling refuses to connect".to_string(),
            ));
        }
        self.publish_connected();
        Ok(())
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.disconnections.fetch_add(1, Ordering::SeqCst);
        let _ = self.event_tx.send(SignalingEvent::Disconnected {
            reason: DisconnectReason::Manual,
        });
        Ok(())
    }

    async fn probe_alive(&self, _timeout: Duration) -> NetworkResult<()> {
        self.probe_calls.fetch_add(1, Ordering::SeqCst);
        self.recovery_attempts.fetch_add(1, Ordering::SeqCst);
        if !self.is_connected() {
            return Err(NetworkError::ConnectionError(
                "fake signaling is disconnected".to_string(),
            ));
        }
        if self.probe_success.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(NetworkError::TimeoutError(
                "fake signaling probe timed out".to_string(),
            ))
        }
    }

    async fn send_register_request(
        &self,
        _request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse> {
        Err(NetworkError::NotImplemented(
            "register request not implemented in fake client".to_string(),
        ))
    }

    async fn send_unregister_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse> {
        Err(NetworkError::NotImplemented(
            "unregister request not implemented in fake client".to_string(),
        ))
    }

    async fn send_heartbeat(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _availability: actr_protocol::ServiceAvailabilityState,
        _power_reserve: f32,
        _mailbox_backlog: f32,
    ) -> NetworkResult<Pong> {
        Err(NetworkError::NotImplemented(
            "heartbeat not implemented in fake client".to_string(),
        ))
    }

    async fn send_route_candidates_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse> {
        Err(NetworkError::NotImplemented(
            "route candidates not implemented in fake client".to_string(),
        ))
    }

    async fn get_signing_key(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _key_id: u32,
    ) -> NetworkResult<(u32, Vec<u8>)> {
        Err(NetworkError::NotImplemented(
            "get_signing_key not implemented in fake client".to_string(),
        ))
    }

    async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
        Err(NetworkError::NotImplemented(
            "send_envelope not implemented in fake client".to_string(),
        ))
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        Err(NetworkError::NotImplemented(
            "receive_envelope not implemented in fake client".to_string(),
        ))
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn get_stats(&self) -> SignalingStats {
        self.stats()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
        self.event_tx.subscribe()
    }

    async fn set_actor_id(&self, _actor_id: ActrId) {}

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}
}

fn snapshot(sequence: u64, availability: NetworkAvailability, wifi: bool) -> NetworkSnapshot {
    NetworkSnapshot {
        sequence,
        availability,
        transport: NetworkTransportFlags {
            wifi,
            cellular: false,
            ethernet: false,
            vpn: false,
            other: false,
        },
        is_expensive: false,
        is_constrained: false,
    }
}

fn online_snapshot(sequence: u64) -> NetworkSnapshot {
    snapshot(sequence, NetworkAvailability::Available, true)
}

fn offline_snapshot(sequence: u64) -> NetworkSnapshot {
    snapshot(sequence, NetworkAvailability::Unavailable, false)
}

/// Send a raw `NetworkEventRequest` with an explicit `source_epoch`, bypassing
/// `NetworkEventHandle` so these tests can control epoch/sequence directly per
/// the RFC-0400 identity model. Returns the acceptance reply.
async fn send_raw(
    event_tx: &tokio::sync::mpsc::Sender<NetworkEventRequest>,
    event: NetworkEvent,
    source_epoch: u64,
) -> actr_hyper::lifecycle::NetworkEventResult {
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    event_tx
        .send(NetworkEventRequest {
            event,
            result_tx,
            source_epoch,
            observed_at: tokio::time::Instant::now(),
        })
        .await
        .expect("request should queue");
    result_rx.await.expect("acceptance reply should arrive")
}

/// Wait for the next signaling event matching `pred`. A real notification —
/// not a sleep — so it also drives a paused clock's timers forward exactly as
/// far as the effect chain requires.
async fn wait_for_event(
    events: &mut broadcast::Receiver<SignalingEvent>,
    mut pred: impl FnMut(&SignalingEvent) -> bool,
) {
    loop {
        let event = events
            .recv()
            .await
            .expect("signaling event stream should stay open for the duration of the test");
        if pred(&event) {
            return;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn inv1_stale_and_duplicate_snapshots_cannot_advance_policy_but_a_new_epoch_does() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let (status_tx, mut status_rx) = watch::channel(SupervisorStatus::default());
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler_with_status(
            event_rx,
            processor,
            reconciler_shutdown,
            status_tx,
        )
        .await;
    });

    // Epoch 1, sequence 5: the first material online transition. Wait for its
    // derived effect to fully settle before probing further, so later
    // (non-material) sends cannot race an in-flight completion.
    send_raw(
        &event_tx,
        NetworkEvent::NetworkPathChanged {
            snapshot: online_snapshot(5),
        },
        1,
    )
    .await;
    status_rx
        .wait_for(|s| s.last_outcome.is_some())
        .await
        .expect("status stream should stay open");
    let r1 = status_rx.borrow().policy_revision;
    assert!(r1 > 0, "the first material snapshot must advance policy");

    // Epoch 1, sequence 3 (lower than the last-accepted 5): stale, discarded.
    send_raw(
        &event_tx,
        NetworkEvent::NetworkPathChanged {
            snapshot: offline_snapshot(3),
        },
        1,
    )
    .await;
    status_rx.changed().await.expect("status stream open");
    assert_eq!(
        status_rx.borrow().policy_revision,
        r1,
        "a stale sequence within the same epoch must not advance policy"
    );

    // Epoch 1, sequence 6 (newer, but structurally identical to the last
    // accepted online snapshot): a duplicate, also inert.
    send_raw(
        &event_tx,
        NetworkEvent::NetworkPathChanged {
            snapshot: online_snapshot(6),
        },
        1,
    )
    .await;
    status_rx.changed().await.expect("status stream open");
    assert_eq!(
        status_rx.borrow().policy_revision,
        r1,
        "a structural duplicate must not advance policy either"
    );

    // Epoch 2, sequence 1: a strictly lower raw sequence number, but a fresh
    // source epoch begins a new monotonic sequence and must be accepted.
    send_raw(
        &event_tx,
        NetworkEvent::NetworkPathChanged {
            snapshot: offline_snapshot(1),
        },
        2,
    )
    .await;
    status_rx.changed().await.expect("status stream open");
    assert!(
        status_rx.borrow().policy_revision > r1,
        "a new source epoch must begin a fresh monotonic sequence and be accepted"
    );

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test(start_paused = true)]
async fn inv2_online_rollback_before_grace_performs_no_disconnect() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    send_raw(
        &event_tx,
        NetworkEvent::NetworkPathChanged {
            snapshot: offline_snapshot(1),
        },
        1,
    )
    .await;
    // Well within the 400ms OfflineCandidate grace window.
    tokio::time::advance(Duration::from_millis(50)).await;
    send_raw(
        &event_tx,
        NetworkEvent::NetworkPathChanged {
            snapshot: online_snapshot(2),
        },
        1,
    )
    .await;

    // Let the *original* candidate's grace deadline pass. It must have been
    // cancelled by the rollback, so no disconnect ever fires.
    tokio::time::advance(Duration::from_millis(500)).await;
    assert!(client.is_connected());
    assert_eq!(
        client.get_stats().disconnections,
        0,
        "a fast rollback before the grace deadline must perform no disconnect"
    );

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test(start_paused = true)]
async fn inv3_offline_candidate_permits_existing_session_without_immediate_disconnect() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    let accepted = send_raw(
        &event_tx,
        NetworkEvent::NetworkPathChanged {
            snapshot: offline_snapshot(1),
        },
        1,
    )
    .await;
    assert!(accepted.success);

    // Immediately after acceptance — no time has been advanced yet — the
    // candidate window must not have torn anything down: existing-session
    // traffic remains admitted and no destructive barrier has been raised.
    assert!(
        client.is_connected(),
        "entering OfflineCandidate must not immediately disconnect"
    );
    assert_eq!(client.get_stats().disconnections, 0);

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test]
async fn inv6_duplicate_foreground_creates_no_recovery_work() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new(event_tx);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    for _ in 0..3 {
        assert!(
            handle
                .handle_app_lifecycle_changed(AppLifecycleState::Foreground {
                    background_duration_ms: 0,
                })
                .await
                .expect("foreground fact should complete")
                .success
        );
    }

    assert_eq!(
        client.connect_once_calls.load(Ordering::SeqCst),
        0,
        "cold and duplicate foreground observations must not create recovery work"
    );
    assert_eq!(client.probe_calls(), 0);

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test(start_paused = true)]
async fn inv10_repeated_availability_failures_back_off_instead_of_hot_looping() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    // Both the probe and the reconnect it falls back to fail from here on, so
    // the recovery effect genuinely fails end to end instead of the failed
    // probe being papered over by a same-cycle rebuild.
    client.set_probe_success(false);
    client.set_connect_should_fail(true);

    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let handle = NetworkEventHandle::new(event_tx);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    // The first available-path observation while already connected drives one
    // failing attempt.
    assert!(
        handle
            .handle_network_path_changed(online_snapshot(1))
            .await
            .expect("accepted")
            .success
    );
    tokio::time::advance(Duration::from_millis(50)).await;
    let first_wave = client.recovery_attempts();
    assert!(first_wave >= 1, "the first failure must be attempted");

    // A generous but still bounded window. A hot loop would rack up an
    // enormous attempt count in this much virtual time (a broken zero-delay
    // retry burns through thousands of attempts in microseconds of real
    // time); a capped, jittered backoff instead compounds — roughly
    // 0.5, 1, 2, 4, 8, 16, 30, 30, 30s — retrying only a handful of times.
    //
    // Advanced in 1s steps rather than one large jump: `tokio::time::advance`
    // wakes timers pending *at the moment it is called*, so a single big jump
    // does not chase a timer that is (re)armed only as a reaction to an
    // earlier one firing within the same call.
    for _ in 0..120 {
        tokio::time::advance(Duration::from_secs(1)).await;
    }
    let after = client.recovery_attempts();
    assert!(
        after > first_wave,
        "an availability failure must eventually retry, not park forever"
    );
    assert!(
        after < 20,
        "backoff must bound the retry rate, not hot-loop: saw {after} attempts in 120s"
    );

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test(start_paused = true)]
async fn inv11_acceptance_does_not_wait_for_effect_completion() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let mut events = client.subscribe_events();
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler(event_rx, processor, reconciler_shutdown).await;
    });

    let accepted = send_raw(
        &event_tx,
        NetworkEvent::NetworkPathChanged {
            snapshot: offline_snapshot(1),
        },
        1,
    )
    .await;
    // The acceptance reply already arrived (by construction: `send_raw` awaits
    // it). At this point zero virtual time has elapsed, so the 400ms
    // OfflineCandidate grace timer — let alone the confirmed-disconnect effect
    // it eventually starts — cannot possibly have run yet.
    assert!(accepted.success);
    assert!(
        client.is_connected(),
        "acceptance must not wait for the effect the fact eventually triggers"
    );

    // The effect does complete later, once the grace timer legitimately
    // elapses.
    wait_for_event(&mut events, |event| {
        matches!(event, SignalingEvent::Disconnected { .. })
    })
    .await;
    assert!(!client.is_connected());

    shutdown.cancel();
    reconciler.await.expect("reconciler task should not panic");
}

#[tokio::test(start_paused = true)]
async fn inv30_shutdown_deadline_terminates_the_supervisor_loop() {
    // RFC-0400 invariant 30 / MAJOR 3: when the shutdown overall deadline fires,
    // the supervisor ends unconditionally. The reconcile loop must break on its
    // own — without an external shutdown-token cancel — instead of re-deriving
    // and restarting cleanup after the deadline.
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");
    let processor = Arc::new(DefaultNetworkEventProcessor::new(client.clone(), None));
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(10);
    let (status_tx, mut status_rx) = watch::channel(SupervisorStatus::default());
    // A shutdown token that is never cancelled: termination must come solely
    // from the shutdown overall deadline.
    let shutdown = tokio_util::sync::CancellationToken::new();
    let processor: Arc<dyn NetworkEventProcessor> = processor;
    let reconciler_shutdown = shutdown.clone();
    let reconciler = tokio::spawn(async move {
        run_network_event_reconciler_with_status(
            event_rx,
            processor,
            reconciler_shutdown,
            status_tx,
        )
        .await;
    });

    // AppTerminating enters Terminating, requests cleanup, and arms the 10s
    // shutdown overall deadline. Let the cleanup effect settle.
    send_raw(
        &event_tx,
        NetworkEvent::CleanupConnections {
            reason: CleanupReason::AppTerminating,
        },
        1,
    )
    .await;
    status_rx
        .wait_for(|s| s.last_outcome.is_some())
        .await
        .expect("cleanup effect should settle");

    // Advance past the shutdown overall deadline. The supervisor terminates
    // unconditionally, so the reconciler task ends on its own.
    tokio::time::advance(Duration::from_secs(11)).await;
    reconciler
        .await
        .expect("reconciler must terminate at the shutdown deadline without an external cancel");
    assert!(
        !shutdown.is_cancelled(),
        "termination must come from the shutdown deadline, not the external token"
    );
}
