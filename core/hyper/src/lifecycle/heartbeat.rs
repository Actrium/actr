//! Heartbeat management for ActrNode
//!
//! This module contains functions for sending periodic heartbeat messages
//! to the signaling server and handling responses.

use crate::ais_client::AisClient;
use crate::lifecycle::CredentialState;
use crate::lifecycle::credential_manager::CredentialManager;
use crate::transport::NetworkError;
use crate::wire::webrtc::gate::WebRtcGate;
use crate::wire::webrtc::{HookCallback, HookEvent, SignalingClient, WebRtcCoordinator};
use actr_protocol::{ActrId, RegisterRequest, ServiceAvailabilityState};
use actr_runtime_mailbox::Mailbox;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio_util::sync::CancellationToken;

/// Convert a `prost_types::Timestamp` expiry to a wall-clock
/// `SystemTime`. Clamps negative seconds to the Unix epoch so downstream
/// `Duration` math stays safe.
fn expiry_to_system_time(expires_at: &prost_types::Timestamp) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(expires_at.seconds.max(0) as u64)
}

/// Invoke a [`HookCallback`], if present, awaiting its completion.
async fn fire_hook(cb: Option<&HookCallback>, event: HookEvent) {
    if let Some(cb) = cb {
        cb(event).await;
    }
}

/// Typical mailbox capacity for backlog ratio calculation
/// A typical_capacity of 1000 means 100 messages = 10% backlog
const TYPICAL_CAPACITY: f32 = 1000.0;

/// Log the first heartbeat failure immediately, then sample long failure runs.
/// With the typical 5s-30s heartbeat interval this keeps offline periods visible
/// without flooding client logs.
const HEARTBEAT_FAILURE_LOG_EVERY: u64 = 12;

fn should_log_heartbeat_failure(consecutive_failures: u64) -> bool {
    consecutive_failures == 1 || consecutive_failures.is_multiple_of(HEARTBEAT_FAILURE_LOG_EVERY)
}

/// Get power reserve, mailbox backlog and calculate service availability
///
/// This function fetches the power reserve from pwrzv and mailbox backlog,
/// then calculates the service availability state based on both metrics.
///
/// # Arguments
/// * `mailbox` - Mailbox instance to get backlog statistics
///
/// # Returns
/// A tuple of (power_reserve, mailbox_backlog, availability) where:
/// - `power_reserve`: Power reserve level from pwrzv (1.0 to 5.0, where higher = more available)
/// - `mailbox_backlog`: Mailbox backlog ratio (0.0 to 1.0, where higher = more backlog)
/// - `availability`: Calculated ServiceAvailabilityState
async fn get_power_reserve_and_availability(
    mailbox: &Arc<dyn Mailbox>,
) -> (f32, f32, ServiceAvailabilityState) {
    // TODO: Ensure the default value is correct
    // Get real power reserve from pwrzv (returns 1.0 to 5.0, where higher = more available)
    let power_reserve = pwrzv::get_power_reserve_level_direct().await.unwrap_or(1.0); // Default to minimum capacity on error

    // Get mailbox backlog from mailbox stats
    // Calculate backlog ratio: (queued + inflight) / typical_capacity
    let mailbox_backlog = match mailbox.status().await {
        Ok(stats) => {
            let total_messages = (stats.queued_messages + stats.inflight_messages) as f32;
            (total_messages / TYPICAL_CAPACITY).min(1.0)
        }
        Err(e) => {
            tracing::warn!("⚠️ Failed to get mailbox stats: {}", e);
            0.0
        }
    };

    // TODO: Improve availability calculation
    // Determine availability based on power reserve and mailbox backlog
    // Power reserve range: 1.0 (worst) to 5.0 (best)
    // Thresholds adjusted for 1.0-5.0 range: 4.2 (80%), 3.0 (50%), 1.8 (20%)
    let availability = if power_reserve > 4.2 && mailbox_backlog < 0.5 {
        ServiceAvailabilityState::Full
    } else if power_reserve > 3.0 && mailbox_backlog < 0.8 {
        ServiceAvailabilityState::Degraded
    } else if power_reserve > 1.8 && mailbox_backlog < 0.95 {
        ServiceAvailabilityState::Overloaded
    } else {
        ServiceAvailabilityState::Unavailable
    };

    (power_reserve, mailbox_backlog, availability)
}

/// Send a single heartbeat and handle the Pong response
///
/// This function sends a heartbeat message to the signaling server,
/// waits for the Pong response, and handles credential warnings if present.
/// If credential has expired (401 error), it triggers the Credential Manager.
///
/// # Arguments
/// * `client` - Signaling client for sending heartbeats
/// * `actor_id` - Actor ID for heartbeat messages
/// * `credential_state` - Shared credential state
/// * `mailbox` - Mailbox instance for backlog statistics
/// * `heartbeat_interval` - Interval between heartbeats (used for timeout calculation)
/// * `credential_manager` - Single-flight renewal manager
///
/// Returns `Some(new_actor_id)` only for legacy hard re-registration paths.
#[allow(clippy::too_many_arguments)]
async fn send_heartbeat_and_handle_response(
    client: &Arc<dyn SignalingClient>,
    actor_id: &ActrId,
    credential_state: &CredentialState,
    mailbox: &Arc<dyn Mailbox>,
    heartbeat_interval: Duration,
    _register_request: &RegisterRequest,
    consecutive_failures: &mut u64,
    _ais_endpoint: &str,
    _realm_secret: Option<&str>,
    credential_manager: Option<&CredentialManager>,
    hook_callback: Option<&HookCallback>,
    _webrtc_coordinator: Option<&Arc<WebRtcCoordinator>>,
    _webrtc_gate: Option<&Arc<WebRtcGate>>,
) -> Option<ActrId> {
    // Get current credential from shared state
    let current_credential = credential_state.credential().await;

    // Get power reserve, mailbox backlog and calculate availability
    let (power_reserve, mailbox_backlog, availability) =
        get_power_reserve_and_availability(mailbox).await;

    let ping_timeout_secs = (heartbeat_interval.as_secs() as f64 * 0.4) as u64;
    let pong_response = tokio::time::timeout(
        Duration::from_secs(ping_timeout_secs),
        client.send_heartbeat(
            actor_id.clone(),
            current_credential.clone(),
            availability,
            power_reserve,
            mailbox_backlog,
        ),
    )
    .await;

    let pong = match pong_response {
        Ok(Ok(pong)) => pong,
        Ok(Err(NetworkError::CredentialExpired(msg))) => {
            // Credential has expired, trigger soft renewal. Do not re-register
            // or close peers from the heartbeat path.
            tracing::warn!(
                "⚠️ Credential expired during heartbeat: {}. Triggering credential renewal.",
                msg
            );

            // Fire `on_credential_expiring` with the last-known expiry
            // timestamp (best-effort — the credential might already be
            // past its advertised `expires_at`, but firing the event
            // gives the workload one final chance to observe the
            // transition before we trigger renewal).
            if let Some(expires_at) = credential_state.expires_at().await {
                fire_hook(
                    hook_callback,
                    HookEvent::CredentialExpiring {
                        new_expiry: expiry_to_system_time(&expires_at),
                    },
                )
                .await;
            }

            if let Some(manager) = credential_manager {
                manager.trigger_renewal();
            } else {
                tracing::warn!(
                    "Credential expired but CredentialManager is not installed; keeping existing identity"
                );
            }
            return None;
        }
        Ok(Err(e)) => {
            *consecutive_failures += 1;
            if should_log_heartbeat_failure(*consecutive_failures) {
                tracing::warn!(
                    consecutive_failures = *consecutive_failures,
                    "⚠️ Failed to send heartbeat or receive Pong: {}",
                    e
                );
            } else {
                tracing::debug!(
                    consecutive_failures = *consecutive_failures,
                    "Suppressed repeated heartbeat failure: {}",
                    e
                );
            }
            return None;
        }
        Err(_) => {
            *consecutive_failures += 1;
            if should_log_heartbeat_failure(*consecutive_failures) {
                tracing::warn!(
                    consecutive_failures = *consecutive_failures,
                    "⚠️ Heartbeat timeout after {}s",
                    ping_timeout_secs
                );
            } else {
                tracing::debug!(
                    consecutive_failures = *consecutive_failures,
                    "Suppressed repeated heartbeat timeout after {}s",
                    ping_timeout_secs
                );
            }
            return None;
        }
    };

    if *consecutive_failures > 0 {
        tracing::info!(
            consecutive_failures = *consecutive_failures,
            "✅ Heartbeat recovered after consecutive failures"
        );
        *consecutive_failures = 0;
    }

    tracing::trace!(
        "💓 Heartbeat sent and Pong received for Actor {} (power_reserve={:.2}, mailbox_backlog={:.2}, availability={:?})",
        actor_id,
        power_reserve,
        mailbox_backlog,
        availability
    );
    // TODO: Handle suggest_interval_secs
    // Handle credential_warning
    if let Some(warning) = pong.credential_warning {
        tracing::warn!(
            "⚠️ Credential warning received: type={:?}, message={}",
            warning.r#type(),
            warning.message
        );

        // Fire `on_credential_expiring` hook once per warning so the
        // workload can preload its credential-renewed handlers (e.g. to
        // rotate derived secrets) before the refresh round-trip lands.
        // The signaling warning is an idempotent trigger source for the
        // Credential Manager; actual renewal goes through POST /ais/renew.
        if let Some(expires_at) = credential_state.expires_at().await {
            fire_hook(
                hook_callback,
                HookEvent::CredentialExpiring {
                    new_expiry: expiry_to_system_time(&expires_at),
                },
            )
            .await;
        }
    }
    None
}

/// Heartbeat task that periodically sends Ping messages to signaling server
///
/// This task runs in a loop, sending heartbeat messages at the specified interval
/// and handling Pong responses, including credential warnings.
/// If credential has expired (401 error), it triggers re-registration.
///
/// # Arguments
/// * `shutdown` - Cancellation token for graceful shutdown
/// * `client` - Signaling client for sending heartbeats
/// * `actor_id` - Actor ID for heartbeat messages
/// * `credential_state` - Shared credential state
/// * `mailbox` - Mailbox instance for backlog statistics
/// * `heartbeat_interval` - Interval between heartbeats
/// * `register_request` - RegisterRequest for re-registration on credential expiry
/// * `ais_endpoint` - AIS HTTP endpoint for re-registration
#[allow(clippy::too_many_arguments)]
pub async fn heartbeat_task(
    shutdown: CancellationToken,
    client: Arc<dyn SignalingClient>,
    actor_id: ActrId,
    credential_state: CredentialState,
    mailbox: Arc<dyn Mailbox>,
    heartbeat_interval: Duration,
    register_request: RegisterRequest,
    ais_endpoint: String,
    realm_secret: Option<String>,
    credential_manager: Option<CredentialManager>,
    hook_callback: Option<HookCallback>,
    webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
    webrtc_gate: Option<Arc<WebRtcGate>>,
) {
    let mut interval = tokio::time::interval(heartbeat_interval);
    let mut actor_id = actor_id;
    let mut consecutive_failures = 0;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                tracing::info!("💓 Heartbeat task received shutdown signal");
                break;
            }
            _ = interval.tick() => {
                if let Some(manager) = credential_manager.as_ref() {
                    actor_id = manager.session_state().actor_id().await;
                }

                if !client.is_connected() {
                    match client.connect_once().await {
                        Ok(()) => {
                            tracing::info!("✅ Signaling reconnected before heartbeat");
                        }
                        Err(e) => {
                            consecutive_failures += 1;
                            if should_log_heartbeat_failure(consecutive_failures) {
                                tracing::warn!(
                                    consecutive_failures,
                                    "⚠️ Failed to reconnect signaling before heartbeat: {}",
                                    e
                                );
                            } else {
                                tracing::debug!(
                                    consecutive_failures,
                                    "Suppressed repeated signaling reconnect failure before heartbeat: {}",
                                    e
                                );
                            }
                            client.schedule_auto_reconnect();
                            continue;
                        }
                    }
                }

                if let Some(new_actor_id) = send_heartbeat_and_handle_response(
                    &client,
                    &actor_id,
                    &credential_state,
                    &mailbox,
                    heartbeat_interval,
                    &register_request,
                    &mut consecutive_failures,
                    &ais_endpoint,
                    realm_secret.as_deref(),
                    credential_manager.as_ref(),
                    hook_callback.as_ref(),
                    webrtc_coordinator.as_ref(),
                    webrtc_gate.as_ref(),
                )
                .await {
                    tracing::info!(
                        "🔄 Heartbeat actor_id updated: {} -> {}",
                        actor_id,
                        new_actor_id,
                    );
                    actor_id = new_actor_id;
                }
            }
        }
    }
    tracing::info!("✅ Heartbeat task terminated gracefully");
}

/// Re-register actor after credential expiry
///
/// When the credential has completely expired, re-register via AIS HTTP,
/// then disconnect/reconnect the signaling WebSocket with the new credential.
///
/// # Arguments
/// * `client` - Signaling client for reconnection
/// * `actor_id` - Current actor ID (used for logging)
/// * `register_request` - RegisterRequest containing actor type, realm, and service spec
/// * `credential_state` - Shared credential state to update
/// * `ais_endpoint` - AIS HTTP endpoint for registration
#[allow(dead_code)]
async fn re_register_task(
    client: Arc<dyn SignalingClient>,
    actor_id: ActrId,
    register_request: RegisterRequest,
    credential_state: CredentialState,
    ais_endpoint: String,
    realm_secret: Option<String>,
    hook_callback: Option<HookCallback>,
) -> ActrId {
    tracing::info!(
        "🔄 Re-registering actor {} after credential expiry via AIS HTTP (type: {}/{})",
        actor_id,
        register_request.actr_type.manufacturer,
        register_request.actr_type.name
    );

    // Step 1: Register via AIS HTTP to get new credential
    let mut ais = AisClient::new(&ais_endpoint);
    if let Some(secret) = realm_secret {
        ais = ais.with_realm_secret(secret);
    }
    let resp = match ais.register_with_manifest(register_request.clone()).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("❌ AIS re-registration HTTP request failed: {}", e);
            return actor_id;
        }
    };

    match resp.result {
        Some(actr_protocol::register_response::Result::Success(register_ok)) => {
            let new_actor_id = register_ok.actr_id.clone();
            let new_credential = register_ok.credential;
            let new_expires_at = register_ok.credential_expires_at;
            let new_turn_credential = Some(register_ok.turn_credential);

            // Update shared credential state
            credential_state
                .update(new_credential.clone(), new_expires_at, new_turn_credential)
                .await;

            // Step 2: Clear old identity from signaling client
            client.clear_identity().await;

            // Step 3: Disconnect old WebSocket session
            tracing::info!("🔌 Disconnecting signaling client to refresh session");
            if let Err(e) = client.disconnect().await {
                tracing::warn!("⚠️ Disconnect failed (non-fatal, continuing): {}", e);
            }

            // Step 4: Update signaling client identity with new credential
            client.set_actor_id(new_actor_id.clone()).await;
            client.set_credential_state(credential_state.clone()).await;

            // Step 5: Reconnect signaling WebSocket (URL will carry new credential)
            tracing::info!("🔗 Reconnecting signaling client with new credential");
            match client.connect_once().await {
                Ok(()) => {
                    tracing::info!(
                        "✅ Re-registration successful and signaling reconnected (ActrId: {})",
                        new_actor_id,
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "❌ AIS re-registration succeeded but signaling reconnect is pending: {}",
                        e
                    );
                    client.schedule_auto_reconnect();
                }
            }

            tracing::debug!("TurnCredential updated, TURN authentication ready");

            if let Some(expires_at) = &new_expires_at {
                tracing::debug!("⏰ New credential expires at: {}s", expires_at.seconds);
                // Fire `on_credential_renewed` for the re-registration
                // path as well — same semantics as the refresh path.
                fire_hook(
                    hook_callback.as_ref(),
                    HookEvent::CredentialRenewed {
                        new_expiry: expiry_to_system_time(expires_at),
                    },
                )
                .await;
            }

            new_actor_id
        }
        Some(actr_protocol::register_response::Result::Error(err)) => {
            tracing::error!(
                "❌ AIS re-registration failed: code={}, message={}",
                err.code,
                err.message
            );
            actor_id
        }
        None => {
            tracing::error!("❌ AIS re-registration response missing result");
            actor_id
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbound::MediaFrameRegistry;
    use crate::transport::{NetworkError, NetworkResult};
    use crate::wire::webrtc::{DisconnectReason, SignalingEvent, SignalingStats, WebRtcConfig};
    use actr_protocol::prost::Message as _;
    use actr_protocol::{
        AIdCredential, ActrType, Pong, Realm, RegisterResponse, RouteCandidatesRequest,
        RouteCandidatesResponse, SignalingEnvelope, TurnCredential, UnregisterResponse,
        register_response,
    };
    use actr_runtime_mailbox::{MailboxStats, MessagePriority, MessageRecord};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::{Notify, broadcast};
    use uuid::Uuid;

    fn test_actor_id(serial_number: u64) -> ActrId {
        ActrId {
            realm: Realm { realm_id: 1 },
            serial_number,
            r#type: ActrType {
                manufacturer: "acme".to_string(),
                name: "node".to_string(),
                version: "1.0.0".to_string(),
            },
        }
    }

    fn test_credential() -> AIdCredential {
        AIdCredential {
            key_id: 7,
            claims: bytes::Bytes::from_static(b"claims"),
            signature: bytes::Bytes::from(vec![0u8; 64]),
        }
    }

    struct EmptyMailbox;

    #[async_trait::async_trait]
    impl Mailbox for EmptyMailbox {
        async fn enqueue(
            &self,
            _from: Vec<u8>,
            _payload: Vec<u8>,
            _priority: MessagePriority,
        ) -> actr_runtime_mailbox::StorageResult<Uuid> {
            unimplemented!("not used by this test")
        }

        async fn dequeue(&self) -> actr_runtime_mailbox::StorageResult<Vec<MessageRecord>> {
            unimplemented!("not used by this test")
        }

        async fn ack(&self, _message_id: Uuid) -> actr_runtime_mailbox::StorageResult<()> {
            unimplemented!("not used by this test")
        }

        async fn status(&self) -> actr_runtime_mailbox::StorageResult<MailboxStats> {
            Ok(MailboxStats {
                queued_messages: 0,
                inflight_messages: 0,
                queued_by_priority: HashMap::new(),
            })
        }
    }

    struct ExpiredHeartbeatSignalingClient {
        event_tx: broadcast::Sender<SignalingEvent>,
    }

    impl ExpiredHeartbeatSignalingClient {
        fn new() -> Self {
            let (event_tx, _rx) = broadcast::channel(16);
            Self { event_tx }
        }
    }

    #[async_trait::async_trait]
    impl SignalingClient for ExpiredHeartbeatSignalingClient {
        async fn connect(&self) -> NetworkResult<()> {
            Ok(())
        }

        async fn connect_once(&self) -> NetworkResult<()> {
            Ok(())
        }

        async fn disconnect(&self) -> NetworkResult<()> {
            let _ = self.event_tx.send(SignalingEvent::Disconnected {
                reason: DisconnectReason::Manual,
            });
            Ok(())
        }

        async fn send_register_request(
            &self,
            _request: RegisterRequest,
        ) -> NetworkResult<RegisterResponse> {
            unimplemented!("not used by this test")
        }

        async fn send_unregister_request(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _reason: Option<String>,
        ) -> NetworkResult<UnregisterResponse> {
            unimplemented!("not used by this test")
        }

        async fn send_heartbeat(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _availability: ServiceAvailabilityState,
            _power_reserve: f32,
            _mailbox_backlog: f32,
        ) -> NetworkResult<Pong> {
            Err(NetworkError::CredentialExpired(
                "Credential validation failed: Invalid credential format".to_string(),
            ))
        }

        async fn send_route_candidates_request(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _request: RouteCandidatesRequest,
        ) -> NetworkResult<RouteCandidatesResponse> {
            unimplemented!("not used by this test")
        }

        async fn get_signing_key(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _key_id: u32,
        ) -> NetworkResult<(u32, Vec<u8>)> {
            unimplemented!("not used by this test")
        }

        async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
            Ok(())
        }

        async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
            Ok(None)
        }

        fn is_connected(&self) -> bool {
            true
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

    struct ReconnectBeforeHeartbeatClient {
        connected: AtomicBool,
        connect_calls: AtomicUsize,
        heartbeat_calls: AtomicUsize,
        heartbeat_sent: Notify,
    }

    impl ReconnectBeforeHeartbeatClient {
        fn new_disconnected() -> Self {
            Self {
                connected: AtomicBool::new(false),
                connect_calls: AtomicUsize::new(0),
                heartbeat_calls: AtomicUsize::new(0),
                heartbeat_sent: Notify::new(),
            }
        }
    }

    #[async_trait::async_trait]
    impl SignalingClient for ReconnectBeforeHeartbeatClient {
        async fn connect(&self) -> NetworkResult<()> {
            self.connect_calls.fetch_add(1, Ordering::SeqCst);
            self.connected.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn connect_once(&self) -> NetworkResult<()> {
            self.connect().await
        }

        async fn disconnect(&self) -> NetworkResult<()> {
            self.connected.store(false, Ordering::SeqCst);
            Ok(())
        }

        async fn send_register_request(
            &self,
            _request: RegisterRequest,
        ) -> NetworkResult<RegisterResponse> {
            unimplemented!("not used by this test")
        }

        async fn send_unregister_request(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _reason: Option<String>,
        ) -> NetworkResult<UnregisterResponse> {
            unimplemented!("not used by this test")
        }

        async fn send_heartbeat(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _availability: ServiceAvailabilityState,
            _power_reserve: f32,
            _mailbox_backlog: f32,
        ) -> NetworkResult<Pong> {
            if !self.connected.load(Ordering::SeqCst) {
                return Err(NetworkError::ConnectionError(
                    "Cannot send: WebSocket not connected".to_string(),
                ));
            }
            self.heartbeat_calls.fetch_add(1, Ordering::SeqCst);
            self.heartbeat_sent.notify_waiters();
            Ok(Pong {
                seq: 0,
                suggest_interval_secs: None,
                credential_warning: None,
            })
        }

        async fn send_route_candidates_request(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _request: RouteCandidatesRequest,
        ) -> NetworkResult<RouteCandidatesResponse> {
            unimplemented!("not used by this test")
        }

        async fn get_signing_key(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _key_id: u32,
        ) -> NetworkResult<(u32, Vec<u8>)> {
            unimplemented!("not used by this test")
        }

        async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
            Ok(())
        }

        async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
            Ok(None)
        }

        fn is_connected(&self) -> bool {
            self.connected.load(Ordering::SeqCst)
        }

        fn get_stats(&self) -> SignalingStats {
            SignalingStats::default()
        }

        fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
            let (_tx, rx) = broadcast::channel(1);
            rx
        }

        async fn set_actor_id(&self, _actor_id: ActrId) {}

        async fn set_credential_state(&self, _credential_state: CredentialState) {}

        async fn clear_identity(&self) {}
    }

    #[tokio::test]
    async fn heartbeat_tick_reconnects_before_sending_when_signaling_is_disconnected() {
        let actor_id = test_actor_id(1);
        let credential_state = CredentialState::new(test_credential(), None, None);
        let client = Arc::new(ReconnectBeforeHeartbeatClient::new_disconnected());
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(heartbeat_task(
            shutdown.clone(),
            client.clone() as Arc<dyn SignalingClient>,
            actor_id.clone(),
            credential_state,
            Arc::new(EmptyMailbox) as Arc<dyn Mailbox>,
            Duration::from_millis(20),
            RegisterRequest {
                actr_type: actor_id.r#type.clone(),
                realm: actor_id.realm,
                ..Default::default()
            },
            "http://127.0.0.1:1".to_string(),
            None,
            None,
            None,
            None,
            None,
        ));

        tokio::time::timeout(Duration::from_secs(5), async {
            while client.heartbeat_calls.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("heartbeat should be sent after reconnect preflight");

        shutdown.cancel();
        task.await.expect("heartbeat task should stop cleanly");

        assert_eq!(client.connect_calls.load(Ordering::SeqCst), 1);
        assert!(client.heartbeat_calls.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn re_registration_updates_webrtc_coordinator_local_id() {
        let initial_id = test_actor_id(1);
        let renewed_id = test_actor_id(2);
        let credential_state = CredentialState::new(test_credential(), None, None);
        let signaling_client = Arc::new(ExpiredHeartbeatSignalingClient::new());
        let coordinator = Arc::new(WebRtcCoordinator::new(
            initial_id.clone(),
            credential_state.clone(),
            signaling_client.clone(),
            WebRtcConfig::default(),
            Arc::new(MediaFrameRegistry::new()),
        ));

        let register_response = RegisterResponse {
            result: Some(register_response::Result::Success(
                register_response::RegisterOk {
                    actr_id: renewed_id.clone(),
                    credential: test_credential(),
                    turn_credential: TurnCredential {
                        username: "1000:actor".to_string(),
                        password: "password".to_string(),
                        expires_at: 1000,
                    },
                    credential_expires_at: Some(prost_types::Timestamp {
                        seconds: 1000,
                        nanos: 0,
                    }),
                    signaling_heartbeat_interval_secs: 30,
                    signing_pubkey: vec![0u8; 32].into(),
                    signing_key_id: 7,
                    renewal_token: None,
                    renewal_token_expires_at: None,
                },
            )),
        };

        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/register")
            .with_status(200)
            .with_header("content-type", "application/x-protobuf")
            .with_body(register_response.encode_to_vec())
            .create_async()
            .await;

        let mut consecutive_failures = 0;
        let updated_id = send_heartbeat_and_handle_response(
            &(signaling_client as Arc<dyn SignalingClient>),
            &initial_id,
            &credential_state,
            &(Arc::new(EmptyMailbox) as Arc<dyn Mailbox>),
            Duration::from_secs(30),
            &RegisterRequest {
                actr_type: renewed_id.r#type.clone(),
                realm: renewed_id.realm,
                ..Default::default()
            },
            &mut consecutive_failures,
            &server.url(),
            None,
            None,
            None,
            Some(&coordinator),
            None,
        )
        .await;

        assert_eq!(updated_id, Some(renewed_id.clone()));
        assert_eq!(coordinator.local_id_for_test(), renewed_id);
    }

    use actr_protocol::RegisterAuthMode;
    use std::sync::Mutex;

    fn test_realm_secret_actor_id(serial_number: u64) -> ActrId {
        ActrId {
            realm: Realm { realm_id: 7 },
            serial_number,
            r#type: ActrType {
                manufacturer: "demo2".to_string(),
                name: "DuplexStreamService".to_string(),
                version: "1.0.0".to_string(),
            },
        }
    }

    fn test_realm_secret_credential(key_id: u32) -> AIdCredential {
        AIdCredential {
            key_id,
            claims: bytes::Bytes::from_static(b"claims"),
            signature: bytes::Bytes::from_static(&[7; 64]),
        }
    }

    #[derive(Default)]
    struct FakeSignalingClient {
        actor_id: Mutex<Option<ActrId>>,
    }

    #[async_trait::async_trait]
    impl SignalingClient for FakeSignalingClient {
        async fn connect(&self) -> NetworkResult<()> {
            Ok(())
        }

        async fn connect_once(&self) -> NetworkResult<()> {
            Ok(())
        }

        async fn disconnect(&self) -> NetworkResult<()> {
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
            Ok(())
        }

        async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
            Ok(None)
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn get_stats(&self) -> SignalingStats {
            SignalingStats::default()
        }

        fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
            let (_tx, rx) = broadcast::channel(1);
            rx
        }

        async fn set_actor_id(&self, actor_id: ActrId) {
            *self.actor_id.lock().expect("actor_id mutex poisoned") = Some(actor_id);
        }

        async fn set_credential_state(&self, _credential_state: CredentialState) {}

        async fn clear_identity(&self) {}
    }

    #[tokio::test]
    async fn re_registration_sends_realm_secret_to_ais() {
        let mut server = mockito::Server::new_async().await;
        let new_actor_id = test_realm_secret_actor_id(42);
        let register_response = RegisterResponse {
            result: Some(register_response::Result::Success(
                register_response::RegisterOk {
                    actr_id: new_actor_id.clone(),
                    credential: test_realm_secret_credential(2),
                    turn_credential: TurnCredential {
                        username: "turn-user".to_string(),
                        password: "turn-password".to_string(),
                        expires_at: 123,
                    },
                    credential_expires_at: Some(prost_types::Timestamp {
                        seconds: 456,
                        nanos: 0,
                    }),
                    signaling_heartbeat_interval_secs: 30,
                    signing_pubkey: bytes::Bytes::from_static(&[1; 32]),
                    signing_key_id: 2,
                    renewal_token: None,
                    renewal_token_expires_at: None,
                },
            )),
        };
        let _mock = server
            .mock("POST", "/register")
            .match_header("x-actrix-realm-secret", "rs_test_secret")
            .with_status(200)
            .with_body(register_response.encode_to_vec())
            .create_async()
            .await;

        let old_actor_id = test_realm_secret_actor_id(1);
        let credential_state = CredentialState::new(
            test_realm_secret_credential(1),
            Some(prost_types::Timestamp {
                seconds: 123,
                nanos: 0,
            }),
            None,
        );
        let register_request = RegisterRequest {
            actr_type: old_actor_id.r#type.clone(),
            realm: old_actor_id.realm,
            auth_mode: Some(RegisterAuthMode::Linked as i32),
            ..Default::default()
        };
        let client = Arc::new(FakeSignalingClient::default());

        let returned_actor_id = re_register_task(
            client,
            old_actor_id,
            register_request,
            credential_state,
            server.url(),
            Some("rs_test_secret".to_string()),
            None,
        )
        .await;

        assert_eq!(returned_actor_id, new_actor_id);
    }
}
