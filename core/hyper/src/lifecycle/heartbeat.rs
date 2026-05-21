//! Heartbeat management for ActrNode
//!
//! This module contains functions for sending periodic heartbeat messages
//! to the signaling server and handling responses.

use crate::ais_client::AisClient;
use crate::lifecycle::CredentialState;
use crate::transport::NetworkError;
use crate::wire::webrtc::SignalingClient;
use crate::wire::webrtc::{HookCallback, HookEvent};
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
/// If credential has expired (401 error), it triggers re-registration.
///
/// # Arguments
/// * `client` - Signaling client for sending heartbeats
/// * `actor_id` - Actor ID for heartbeat messages
/// * `credential_state` - Shared credential state
/// * `mailbox` - Mailbox instance for backlog statistics
/// * `heartbeat_interval` - Interval between heartbeats (used for timeout calculation)
/// * `register_request` - RegisterRequest for re-registration on credential expiry
///
/// Returns `Some(new_actor_id)` when re-registration assigns a new ActrId,
/// so the caller can update its loop variable for subsequent heartbeats.
#[allow(clippy::too_many_arguments)]
async fn send_heartbeat_and_handle_response(
    client: &Arc<dyn SignalingClient>,
    actor_id: &ActrId,
    credential_state: &CredentialState,
    mailbox: &Arc<dyn Mailbox>,
    heartbeat_interval: Duration,
    register_request: &RegisterRequest,
    ais_endpoint: &str,
    hook_callback: Option<&HookCallback>,
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
            // Credential has expired, trigger re-registration
            tracing::warn!(
                "⚠️ Credential expired during heartbeat: {}. Attempting re-registration.",
                msg
            );

            // Fire `on_credential_expiring` with the last-known expiry
            // timestamp (best-effort — the credential might already be
            // past its advertised `expires_at`, but firing the event
            // gives the workload one final chance to observe the
            // transition before we attempt re-registration).
            if let Some(expires_at) = credential_state.expires_at().await {
                fire_hook(
                    hook_callback,
                    HookEvent::CredentialExpiring {
                        new_expiry: expiry_to_system_time(&expires_at),
                    },
                )
                .await;
            }

            let new_actor_id = re_register_task(
                client.clone(),
                actor_id.clone(),
                register_request.clone(),
                credential_state.clone(),
                ais_endpoint.to_string(),
                hook_callback.cloned(),
            )
            .await;

            // Return updated ActrId only if it actually changed
            if &new_actor_id != actor_id {
                return Some(new_actor_id);
            }
            return None;
        }
        Ok(Err(e)) => {
            tracing::warn!("⚠️ Failed to send heartbeat or receive Pong: {}", e);
            return None;
        }
        Err(_) => {
            tracing::warn!("⚠️ Heartbeat timeout after {}s", ping_timeout_secs);
            return None;
        }
    };

    tracing::debug!(
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
        if let Some(expires_at) = credential_state.expires_at().await {
            fire_hook(
                hook_callback,
                HookEvent::CredentialExpiring {
                    new_expiry: expiry_to_system_time(&expires_at),
                },
            )
            .await;
        }

        // Trigger immediate credential refresh in a spawned task
        tokio::spawn(credential_refresh_task(
            client.clone(),
            actor_id.clone(),
            credential_state.clone(),
            hook_callback.cloned(),
        ));
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
    hook_callback: Option<HookCallback>,
) {
    let mut interval = tokio::time::interval(heartbeat_interval);
    let mut actor_id = actor_id;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                tracing::info!("💓 Heartbeat task received shutdown signal");
                break;
            }
            _ = interval.tick() => {
                if let Some(new_actor_id) = send_heartbeat_and_handle_response(
                    &client,
                    &actor_id,
                    &credential_state,
                    &mailbox,
                    heartbeat_interval,
                    &register_request,
                    &ais_endpoint,
                    hook_callback.as_ref(),
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

/// Refresh credential for an actor
///
/// This function sends a credential update request to the signaling server
/// and updates the shared credential state upon success.
///
/// # Arguments
/// * `client` - Signaling client for sending credential update request
/// * `actor_id` - Actor ID for the credential update
/// * `credential_state` - Shared credential state to update
async fn credential_refresh_task(
    client: Arc<dyn SignalingClient>,
    actor_id: ActrId,
    credential_state: CredentialState,
    hook_callback: Option<HookCallback>,
) {
    tracing::info!("🔑 Refreshing credential for Actor {}", actor_id);

    match client
        .send_credential_update_request(actor_id.clone(), credential_state.credential().await)
        .await
    {
        Ok(register_response) => {
            match register_response.result {
                Some(actr_protocol::register_response::Result::Success(register_ok)) => {
                    let new_credential = register_ok.credential;
                    let new_expires_at = register_ok.credential_expires_at;
                    // TurnCredential is a required proto field; wrap as Some directly
                    let new_turn_credential = Some(register_ok.turn_credential);

                    // Update shared credential state, synchronously updating TURN credential
                    credential_state
                        .update(new_credential.clone(), new_expires_at, new_turn_credential)
                        .await;

                    tracing::info!(
                        "✅ Credential refreshed successfully for Actor {}",
                        actor_id,
                    );

                    tracing::debug!("TurnCredential updated, TURN authentication ready");

                    if let Some(expires_at) = &new_expires_at {
                        tracing::debug!("⏰ New credential expires at: {}s", expires_at.seconds);
                        // Fire `on_credential_renewed` so the workload
                        // can rotate downstream state tied to the old
                        // credential (e.g. AIS-derived tokens).
                        fire_hook(
                            hook_callback.as_ref(),
                            HookEvent::CredentialRenewed {
                                new_expiry: expiry_to_system_time(expires_at),
                            },
                        )
                        .await;
                    }
                }
                Some(actr_protocol::register_response::Result::Error(err)) => {
                    tracing::error!(
                        "❌ Credential refresh failed: code={}, message={}",
                        err.code,
                        err.message
                    );
                }
                None => {
                    tracing::error!("❌ Credential refresh response missing result");
                }
            }
        }
        Err(e) => {
            tracing::warn!("⚠️ Failed to send credential update request: {}", e);
        }
    }
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
async fn re_register_task(
    client: Arc<dyn SignalingClient>,
    actor_id: ActrId,
    register_request: RegisterRequest,
    credential_state: CredentialState,
    ais_endpoint: String,
    hook_callback: Option<HookCallback>,
) -> ActrId {
    tracing::info!(
        "🔄 Re-registering actor {} after credential expiry via AIS HTTP (type: {}/{})",
        actor_id,
        register_request.actr_type.manufacturer,
        register_request.actr_type.name
    );

    // Step 1: Register via AIS HTTP to get new credential
    let ais = AisClient::new(&ais_endpoint);
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
            if let Err(e) = client.connect().await {
                tracing::error!("❌ Failed to reconnect after re-registration: {}", e);
                // Credential is updated even if reconnect fails — next heartbeat retry will reconnect
            }

            tracing::info!(
                "✅ Re-registration successful via AIS HTTP (ActrId: {})",
                new_actor_id,
            );

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
