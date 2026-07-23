//! VNet black-box coverage for the responsive recovery supervisor.
//!
//! These tests run real signaling, WebRTC, PeerGate and VNet resources while
//! driving lifecycle inputs through `NetworkEventHandle` plus the normalized
//! fact channel used by the production node.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use actr_hyper::lifecycle::{
    AppLifecycleState, CleanupReason, DefaultNetworkEventProcessor, NetworkAvailability,
    NetworkEventProcessor, NetworkRecoveryAction, NetworkSnapshot, NetworkTransportFlags,
    SignalingFactLostCause, SignalingFactOrigin, TeardownReport,
};
use actr_hyper::test_support::{
    TestHarness, spawn_gated_network_event_supervisor, spawn_network_event_supervisor,
};
use actr_protocol::ActrId;
use tokio::sync::Notify;

struct RpcTasks(Vec<tokio::task::JoinHandle<()>>);

impl Drop for RpcTasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

struct RecordingProcessor {
    inner: Arc<DefaultNetworkEventProcessor>,
    actions: StdMutex<Vec<NetworkRecoveryAction>>,
}

struct BlockingRecoveryProcessor {
    inner: Arc<DefaultNetworkEventProcessor>,
    actions: StdMutex<Vec<NetworkRecoveryAction>>,
    block_action: NetworkRecoveryAction,
    block_once: AtomicBool,
    blocked: Notify,
    release: Notify,
}

impl BlockingRecoveryProcessor {
    fn new(inner: Arc<DefaultNetworkEventProcessor>, block_action: NetworkRecoveryAction) -> Self {
        Self {
            inner,
            actions: StdMutex::new(Vec::new()),
            block_action,
            block_once: AtomicBool::new(true),
            blocked: Notify::new(),
            release: Notify::new(),
        }
    }

    fn actions(&self) -> Vec<NetworkRecoveryAction> {
        self.actions.lock().expect("actions mutex poisoned").clone()
    }

    async fn wait_until_blocked(&self) {
        tokio::time::timeout(Duration::from_secs(10), self.blocked.notified())
            .await
            .expect("recovery action did not reach the deterministic block point");
    }

    fn release(&self) {
        self.release.notify_one();
    }
}

impl RecordingProcessor {
    fn new(inner: Arc<DefaultNetworkEventProcessor>) -> Self {
        Self {
            inner,
            actions: StdMutex::new(Vec::new()),
        }
    }

    fn actions(&self) -> Vec<NetworkRecoveryAction> {
        self.actions.lock().expect("actions mutex poisoned").clone()
    }

    fn record(&self, action: NetworkRecoveryAction) {
        self.actions
            .lock()
            .expect("actions mutex poisoned")
            .push(action);
    }
}

#[async_trait::async_trait]
impl NetworkEventProcessor for RecordingProcessor {
    fn suppress_auto_reconnect(&self) {
        self.inner.suppress_auto_reconnect();
    }

    fn resume_auto_reconnect(&self) {
        self.inner.resume_auto_reconnect();
    }

    async fn process_network_available(&self) -> Result<(), String> {
        self.inner.process_network_available().await
    }

    async fn process_network_lost(&self) -> Result<(), String> {
        self.inner.process_network_lost().await
    }

    async fn process_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<(), String> {
        self.inner
            .process_network_type_changed(is_wifi, is_cellular)
            .await
    }

    async fn cleanup_connections(&self) -> Result<(), String> {
        self.inner.cleanup_connections().await
    }

    async fn probe_connectivity(&self) -> Result<(), String> {
        self.inner.probe_connectivity().await
    }

    async fn force_reconnect(&self) -> Result<(), String> {
        self.inner.force_reconnect().await
    }

    async fn process_network_recovery_action(
        &self,
        action: NetworkRecoveryAction,
    ) -> Result<(), String> {
        self.record(action);
        self.inner.process_network_recovery_action(action).await
    }

    async fn run_bounded_teardown(
        &self,
        action: NetworkRecoveryAction,
        budget: Duration,
    ) -> TeardownReport {
        self.record(action);
        self.inner.run_bounded_teardown(action, budget).await
    }
}

#[async_trait::async_trait]
impl NetworkEventProcessor for BlockingRecoveryProcessor {
    fn suppress_auto_reconnect(&self) {
        self.inner.suppress_auto_reconnect();
    }

    fn resume_auto_reconnect(&self) {
        self.inner.resume_auto_reconnect();
    }

    async fn process_network_available(&self) -> Result<(), String> {
        self.inner.process_network_available().await
    }

    async fn process_network_lost(&self) -> Result<(), String> {
        self.inner.process_network_lost().await
    }

    async fn process_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<(), String> {
        self.inner
            .process_network_type_changed(is_wifi, is_cellular)
            .await
    }

    async fn cleanup_connections(&self) -> Result<(), String> {
        self.inner.cleanup_connections().await
    }

    async fn probe_connectivity(&self) -> Result<(), String> {
        self.inner.probe_connectivity().await
    }

    async fn force_reconnect(&self) -> Result<(), String> {
        self.inner.force_reconnect().await
    }

    async fn process_network_recovery_action(
        &self,
        action: NetworkRecoveryAction,
    ) -> Result<(), String> {
        self.actions
            .lock()
            .expect("actions mutex poisoned")
            .push(action);
        if action == self.block_action && self.block_once.swap(false, Ordering::SeqCst) {
            self.blocked.notify_one();
            self.release.notified().await;
        }
        self.inner.process_network_recovery_action(action).await
    }

    async fn run_bounded_teardown(
        &self,
        action: NetworkRecoveryAction,
        budget: Duration,
    ) -> TeardownReport {
        self.actions
            .lock()
            .expect("actions mutex poisoned")
            .push(action);
        self.inner.run_bounded_teardown(action, budget).await
    }
}

fn snapshot(sequence: u64, availability: NetworkAvailability, wifi: bool) -> NetworkSnapshot {
    NetworkSnapshot {
        sequence,
        availability,
        transport: NetworkTransportFlags {
            wifi,
            cellular: !wifi && availability == NetworkAvailability::Available,
            ethernet: false,
            vpn: false,
            other: false,
        },
        is_expensive: false,
        is_constrained: false,
    }
}

async fn setup_bidirectional_vnet() -> (TestHarness, RpcTasks) {
    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;
    let tasks = RpcTasks(vec![
        harness
            .peer(100)
            .start_rpc_dispatcher("vnet_supervisor_100"),
        harness
            .peer(200)
            .start_rpc_dispatcher("vnet_supervisor_200"),
    ]);

    expect_rpc(&harness, 100, 200, "vnet_setup_100_200").await;
    expect_rpc(&harness, 200, 100, "vnet_setup_200_100").await;
    (harness, tasks)
}

async fn expect_rpc(harness: &TestHarness, from: u64, to: u64, prefix: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let request = harness
            .peer(from)
            .spawn_request(to, &format!("{prefix}_{attempt}"), 2_000);
        match tokio::time::timeout(Duration::from_secs(3), request).await {
            Ok(Ok(Ok(response))) => {
                assert!(!response.is_empty(), "{prefix} returned an empty response");
                return;
            }
            Ok(Ok(Err(error))) => {
                let message = error.to_string();
                assert!(
                    message.contains("connection not ready")
                        || message.contains("Request timeout")
                        || message.contains("timed out")
                        || message.contains("Connection")
                        || message.contains("all transport candidates exhausted"),
                    "{prefix} failed with an unexpected error: {message}"
                );
            }
            Ok(Err(error)) => panic!("{prefix} request task panicked: {error}"),
            Err(_) => {}
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{prefix} did not recover before the deadline"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn prime_live_generation(
    handle: &actr_hyper::lifecycle::NetworkEventHandle,
    sink: &actr_hyper::lifecycle::SupervisorFactSink,
) {
    sink.signaling_generation_committed(1, SignalingFactOrigin::External);
    // Internal inputs are selected before public events. Acceptance of this
    // duplicate foreground observation therefore proves the queued generation
    // fact has already reached policy state.
    let accepted = handle
        .handle_app_lifecycle_changed(AppLifecycleState::Foreground {
            background_duration_ms: 0,
        })
        .await
        .expect("priming foreground observation should be accepted");
    assert!(accepted.success);
}

async fn wait_for_actions(processor: &RecordingProcessor, expected: &[NetworkRecoveryAction]) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let actions = processor.actions();
        if actions == expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected actions {expected:?}, got {actions:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_for_recovery_session(harness: &TestHarness, serial: u64, peer_id: &ActrId) -> u64 {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = harness
            .peer(serial)
            .coordinator
            .peer_recovery_status(peer_id)
            .await
        {
            return status.session_id;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "peer recovery guard did not become active"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn expect_connection_not_ready(
    request: tokio::task::JoinHandle<actr_protocol::ActorResult<actr_framework::Bytes>>,
) {
    match tokio::time::timeout(Duration::from_secs(3), request).await {
        Ok(Ok(Err(error))) => assert!(
            error.to_string().contains("connection not ready"),
            "expected ConnectionNotReady, got {error}"
        ),
        Ok(Ok(Ok(response))) => panic!(
            "request unexpectedly succeeded with {} response bytes",
            response.len()
        ),
        Ok(Err(error)) => panic!("request task panicked: {error}"),
        Err(_) => panic!("ConnectionNotReady request did not fail fast"),
    }
}

async fn start_blocked_restore(
    handle: &actr_hyper::lifecycle::NetworkEventHandle,
    sink: &actr_hyper::lifecycle::SupervisorFactSink,
    processor: &BlockingRecoveryProcessor,
) {
    sink.signaling_generation_lost(1, SignalingFactLostCause::RemoteReset);
    let accepted = handle
        .handle_network_path_changed(snapshot(1, NetworkAvailability::Available, true))
        .await
        .expect("online route change should be accepted");
    assert!(accepted.success);
    processor.wait_until_blocked().await;
    assert_eq!(processor.actions(), vec![NetworkRecoveryAction::Restore]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn short_foreground_preserves_inflight_restore_and_recovers_vnet_bidirectionally() {
    let (harness, _rpc_tasks) = setup_bidirectional_vnet().await;
    let target = harness.peer(200).id.clone();
    let initial_session = harness
        .peer(100)
        .coordinator
        .get_peer_session_id(&target)
        .await
        .expect("initial WebRTC session should exist");

    let processor = Arc::new(BlockingRecoveryProcessor::new(
        harness.peer(100).network_processor(),
        NetworkRecoveryAction::Restore,
    ));
    let processor_for_task: Arc<dyn NetworkEventProcessor> = processor.clone();
    let (handle, sink, shutdown, reconciler) =
        spawn_gated_network_event_supervisor(processor_for_task);
    harness
        .peer(100)
        .signaling_client
        .set_supervisor_fact_sink(sink.clone());
    prime_live_generation(&handle, &sink).await;

    start_blocked_restore(&handle, &sink, &processor).await;
    let background = handle
        .handle_app_lifecycle_changed(AppLifecycleState::Background)
        .await
        .expect("background should be accepted while Restore is running");
    assert!(background.success);
    let foreground = handle
        .handle_app_lifecycle_changed(AppLifecycleState::Foreground {
            background_duration_ms: 5_000,
        })
        .await
        .expect("short foreground should be accepted while Restore is running");
    assert!(foreground.success);
    tokio::task::yield_now().await;
    assert_eq!(
        processor.actions(),
        vec![NetworkRecoveryAction::Restore],
        "short foreground must neither cancel nor duplicate the in-flight Restore"
    );

    processor.release();
    expect_rpc(&harness, 100, 200, "short_foreground_restore_100_200").await;
    expect_rpc(&harness, 200, 100, "short_foreground_restore_200_100").await;
    assert_eq!(
        harness
            .peer(100)
            .coordinator
            .get_peer_session_id(&target)
            .await,
        Some(initial_session),
        "short foreground should preserve the ICE-restarted WebRTC session"
    );
    assert_eq!(harness.peer(100).pending_count().await, 0);
    assert_eq!(harness.peer(200).pending_count().await, 0);

    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn long_foreground_preempts_inflight_restore_and_rebuilds_vnet_bidirectionally() {
    let (harness, _rpc_tasks) = setup_bidirectional_vnet().await;
    let target = harness.peer(200).id.clone();
    let initial_session = harness
        .peer(100)
        .coordinator
        .get_peer_session_id(&target)
        .await
        .expect("initial WebRTC session should exist");

    let processor = Arc::new(BlockingRecoveryProcessor::new(
        harness.peer(100).network_processor(),
        NetworkRecoveryAction::Restore,
    ));
    let processor_for_task: Arc<dyn NetworkEventProcessor> = processor.clone();
    let (handle, sink, shutdown, reconciler) =
        spawn_gated_network_event_supervisor(processor_for_task);
    harness
        .peer(100)
        .signaling_client
        .set_supervisor_fact_sink(sink.clone());
    prime_live_generation(&handle, &sink).await;

    start_blocked_restore(&handle, &sink, &processor).await;
    let background = handle
        .handle_app_lifecycle_changed(AppLifecycleState::Background)
        .await
        .expect("background should be accepted while Restore is running");
    assert!(background.success);
    let foreground = handle
        .handle_app_lifecycle_changed(AppLifecycleState::Foreground {
            background_duration_ms: 60_001,
        })
        .await
        .expect("long foreground should be accepted while Restore is running");
    assert!(foreground.success);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let actions = processor.actions();
        if actions
            == vec![
                NetworkRecoveryAction::Restore,
                NetworkRecoveryAction::ForceReconnect,
            ]
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "long foreground did not preempt Restore with Reconnect: {actions:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    expect_rpc(&harness, 100, 200, "long_foreground_reconnect_100_200").await;
    expect_rpc(&harness, 200, 100, "long_foreground_reconnect_200_100").await;
    let rebuilt_session = harness
        .peer(100)
        .coordinator
        .get_peer_session_id(&target)
        .await
        .expect("long foreground should create a replacement WebRTC session");
    assert_ne!(
        rebuilt_session, initial_session,
        "long foreground must not revive the pre-Reconnect WebRTC session"
    );
    assert_eq!(harness.peer(100).pending_count().await, 0);
    assert_eq!(harness.peer(200).pending_count().await, 0);

    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vnet_offline_rollback_before_grace_keeps_session_and_sends_no_sdp() {
    let (harness, _rpc_tasks) = setup_bidirectional_vnet().await;
    let target = harness.peer(200).id.clone();
    let initial_session = harness
        .peer(100)
        .coordinator
        .get_peer_session_id(&target)
        .await
        .expect("initial WebRTC session should exist");

    let inner = harness.peer(100).network_processor();
    let processor = Arc::new(RecordingProcessor::new(inner));
    let processor_for_task: Arc<dyn NetworkEventProcessor> = processor.clone();
    let (handle, sink, shutdown, reconciler) = spawn_network_event_supervisor(processor_for_task);
    harness
        .peer(100)
        .signaling_client
        .set_supervisor_fact_sink(sink.clone());
    prime_live_generation(&handle, &sink).await;
    harness.reset_counters();

    harness.simulate_disconnect();
    let started = Instant::now();
    assert!(
        handle
            .handle_network_path_changed(snapshot(1, NetworkAvailability::Unavailable, false,))
            .await
            .expect("offline candidate should be accepted")
            .success
    );
    harness.simulate_reconnect();
    assert!(
        handle
            .handle_network_path_changed(snapshot(2, NetworkAvailability::Available, true))
            .await
            .expect("online rollback should be accepted")
            .success
    );
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "rollback input must arrive inside offline grace"
    );

    wait_for_actions(&processor, &[NetworkRecoveryAction::Probe]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        processor.actions(),
        vec![NetworkRecoveryAction::Probe],
        "cancelled grace must never run the offline teardown"
    );
    assert_eq!(
        harness.ice_restart_count(),
        0,
        "rollback probe sends no SDP"
    );
    assert_eq!(
        harness
            .peer(100)
            .coordinator
            .get_peer_session_id(&target)
            .await,
        Some(initial_session),
        "fast rollback must retain the current WebRTC session"
    );
    expect_rpc(&harness, 100, 200, "rollback_100_200").await;
    expect_rpc(&harness, 200, 100, "rollback_200_100").await;

    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vnet_committed_offline_restores_once_and_recovers_bidirectionally() {
    let (harness, _rpc_tasks) = setup_bidirectional_vnet().await;
    let target = harness.peer(200).id.clone();
    let initial_session = harness
        .peer(100)
        .coordinator
        .get_peer_session_id(&target)
        .await
        .expect("initial WebRTC session should exist");

    let inner = harness.peer(100).network_processor();
    let processor = Arc::new(RecordingProcessor::new(inner));
    let processor_for_task: Arc<dyn NetworkEventProcessor> = processor.clone();
    let (handle, sink, shutdown, reconciler) = spawn_network_event_supervisor(processor_for_task);
    harness
        .peer(100)
        .signaling_client
        .set_supervisor_fact_sink(sink.clone());
    prime_live_generation(&handle, &sink).await;

    harness.simulate_disconnect();
    handle
        .handle_network_path_changed(snapshot(1, NetworkAvailability::Unavailable, false))
        .await
        .expect("offline candidate should be accepted");
    wait_for_actions(&processor, &[NetworkRecoveryAction::Offline]).await;
    assert_eq!(
        harness.peer(100).transport_manager.dest_count().await,
        1,
        "confirmed offline keeps the guarded WebRTC transport available for ICE recovery"
    );
    wait_for_recovery_session(&harness, 100, &target).await;

    harness.simulate_reconnect();
    handle
        .handle_network_path_changed(snapshot(2, NetworkAvailability::Available, true))
        .await
        .expect("online recovery should be accepted");
    wait_for_actions(
        &processor,
        &[
            NetworkRecoveryAction::Offline,
            NetworkRecoveryAction::Restore,
        ],
    )
    .await;

    expect_rpc(&harness, 100, 200, "committed_restore_100_200").await;
    expect_rpc(&harness, 200, 100, "committed_restore_200_100").await;
    let recovered_session = harness
        .peer(100)
        .coordinator
        .get_peer_session_id(&target)
        .await
        .expect("restored WebRTC session should exist");
    assert_eq!(
        recovered_session, initial_session,
        "confirmed offline recovery should ICE-restart the guarded session"
    );
    assert_eq!(
        processor.actions(),
        vec![
            NetworkRecoveryAction::Offline,
            NetworkRecoveryAction::Restore,
        ],
        "committed offline recovery must restore exactly once"
    );
    assert_eq!(harness.peer(100).pending_count().await, 0);
    assert_eq!(harness.peer(200).pending_count().await, 0);

    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_event_while_webrtc_not_ready_fails_fast_without_pending_leak() {
    let (harness, _rpc_tasks) = setup_bidirectional_vnet().await;
    let target = harness.peer(200).id.clone();
    let processor = harness.peer(100).network_processor();
    let processor_for_task: Arc<dyn NetworkEventProcessor> = processor;
    let (handle, sink, shutdown, reconciler) = spawn_network_event_supervisor(processor_for_task);
    harness
        .peer(100)
        .signaling_client
        .set_supervisor_fact_sink(sink.clone());
    prime_live_generation(&handle, &sink).await;

    harness
        .vnet
        .as_ref()
        .expect("test requires VNet")
        .block_network();
    sink.signaling_generation_lost(1, SignalingFactLostCause::RemoteReset);
    let accepted = handle
        .handle_network_path_changed(snapshot(1, NetworkAvailability::Available, true))
        .await
        .expect("network snapshot should be accepted while recovery starts");
    assert!(accepted.success);
    wait_for_recovery_session(&harness, 100, &target).await;

    let early = harness
        .peer(100)
        .spawn_request(200, "accepted_but_webrtc_not_ready", 30_000);
    expect_connection_not_ready(early).await;
    assert_eq!(
        harness.peer(100).pending_count().await,
        0,
        "fast ConnectionNotReady must not leak pending state"
    );

    harness
        .vnet
        .as_ref()
        .expect("test requires VNet")
        .unblock_network();
    expect_rpc(&harness, 100, 200, "accepted_then_ready_100_200").await;
    expect_rpc(&harness, 200, 100, "accepted_then_ready_200_100").await;

    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cleanup_request_and_recovery_overlap_cannot_revive_stale_session() {
    let (harness, _rpc_tasks) = setup_bidirectional_vnet().await;
    let target = harness.peer(200).id.clone();
    let initial_session = harness
        .peer(100)
        .coordinator
        .get_peer_session_id(&target)
        .await
        .expect("initial WebRTC session should exist");
    let processor = harness.peer(100).network_processor();
    let processor_for_task: Arc<dyn NetworkEventProcessor> = processor;
    let (handle, sink, shutdown, reconciler) = spawn_network_event_supervisor(processor_for_task);
    harness
        .peer(100)
        .signaling_client
        .set_supervisor_fact_sink(sink.clone());
    prime_live_generation(&handle, &sink).await;

    harness
        .vnet
        .as_ref()
        .expect("test requires VNet")
        .block_network();
    sink.signaling_generation_lost(1, SignalingFactLostCause::RemoteReset);
    handle
        .handle_network_path_changed(snapshot(1, NetworkAvailability::Available, true))
        .await
        .expect("recovery event should be accepted");
    wait_for_recovery_session(&harness, 100, &target).await;

    let overlapping_request =
        harness
            .peer(100)
            .spawn_request(200, "request_overlapping_cleanup", 30_000);
    let cleanup = handle
        .cleanup_connections(CleanupReason::ManualReset)
        .await
        .expect("cleanup should be accepted while recovery is active");
    assert!(cleanup.success);
    let online_after_cleanup = handle
        .handle_network_path_changed(snapshot(2, NetworkAvailability::Available, false))
        .await
        .expect("post-cleanup recovery fact should be accepted");
    assert!(online_after_cleanup.success);
    expect_connection_not_ready(overlapping_request).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let dests = harness.peer(100).transport_manager.dest_count().await;
        let guard = harness
            .peer(100)
            .coordinator
            .peer_recovery_status(&target)
            .await;
        if dests == 0 && guard.is_none() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "cleanup did not retire the stale session: dests={dests}, guard={guard:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(harness.peer(100).pending_count().await, 0);

    harness
        .vnet
        .as_ref()
        .expect("test requires VNet")
        .unblock_network();
    expect_rpc(&harness, 100, 200, "cleanup_overlap_recovered_100_200").await;
    expect_rpc(&harness, 200, 100, "cleanup_overlap_recovered_200_100").await;
    let new_session = harness
        .peer(100)
        .coordinator
        .get_peer_session_id(&target)
        .await
        .expect("post-cleanup session should exist");
    assert_ne!(
        new_session, initial_session,
        "cleanup must not revive the pre-cleanup WebRTC session"
    );
    assert_eq!(harness.peer(100).pending_count().await, 0);
    assert_eq!(harness.peer(200).pending_count().await, 0);

    shutdown.cancel();
    reconciler.await.expect("reconciler should stop cleanly");
    harness.shutdown().await;
}
