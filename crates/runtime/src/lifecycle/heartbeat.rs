//! Heartbeat management for ActrNode
//!
//! This module contains functions for sending periodic heartbeat messages
//! to the signaling server and handling responses.

use crate::lifecycle::CredentialState;
use crate::transport::error::NetworkError;
use crate::wire::webrtc::SignalingClient;
use actr_mailbox::Mailbox;
use actr_protocol::{ActrId, ActrIdExt, RegisterRequest, ServiceAvailabilityState};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

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
async fn send_heartbeat_and_handle_response(
    client: &Arc<dyn SignalingClient>,
    actor_id: &ActrId,
    credential_state: &CredentialState,
    mailbox: &Arc<dyn Mailbox>,
    heartbeat_interval: Duration,
    register_request: &RegisterRequest,
) {
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
            tokio::spawn(re_register_task(
                client.clone(),
                register_request.clone(),
                credential_state.clone(),
            ));
            return;
        }
        Ok(Err(e)) => {
            tracing::warn!("⚠️ Failed to send heartbeat or receive Pong: {}", e);
            return;
        }
        Err(_) => {
            tracing::warn!("⚠️ Heartbeat timeout after {}s", ping_timeout_secs);
            return;
        }
    };

    tracing::debug!(
        "💓 Heartbeat sent and Pong received for Actor {} (power_reserve={:.2}, mailbox_backlog={:.2}, availability={:?})",
        actor_id.to_string_repr(),
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

        // Trigger immediate credential refresh in a spawned task
        tokio::spawn(credential_refresh_task(
            client.clone(),
            actor_id.clone(),
            credential_state.clone(),
        ));
    }
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
pub async fn heartbeat_task(
    shutdown: CancellationToken,
    client: Arc<dyn SignalingClient>,
    actor_id: ActrId,
    credential_state: CredentialState,
    mailbox: Arc<dyn Mailbox>,
    heartbeat_interval: Duration,
    register_request: RegisterRequest,
) {
    let mut interval = tokio::time::interval(heartbeat_interval);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                tracing::info!("💓 Heartbeat task received shutdown signal");
                break;
            }
            _ = interval.tick() => {
                send_heartbeat_and_handle_response(
                    &client,
                    &actor_id,
                    &credential_state,
                    &mailbox,
                    heartbeat_interval,
                    &register_request,
                )
                .await;
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
) {
    tracing::info!(
        "🔑 Refreshing credential for Actor {}",
        actor_id.to_string_repr()
    );

    match client
        .send_credential_update_request(actor_id.clone(), credential_state.credential().await)
        .await
    {
        Ok(register_response) => {
            match register_response.result {
                Some(actr_protocol::register_response::Result::Success(register_ok)) => {
                    let new_credential = register_ok.credential;
                    let new_expires_at = register_ok.credential_expires_at;
                    let new_psk = register_ok.psk;

                    // Update shared state including PSK
                    credential_state
                        .update(new_credential.clone(), new_expires_at, new_psk.clone())
                        .await;

                    tracing::info!(
                        "✅ Credential refreshed successfully for Actor {} (new key_id: {})",
                        actor_id.serial_number,
                        new_credential.token_key_id
                    );

                    if new_psk.is_some() {
                        tracing::debug!("🔑 PSK updated for TURN authentication");
                    }

                    if let Some(expires_at) = &new_expires_at {
                        tracing::debug!("⏰ New credential expires at: {}s", expires_at.seconds);
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
/// This function sends a register request to the signaling server
/// and updates the shared credential state upon success.
/// This is triggered when the credential has completely expired (beyond tolerance period)
/// and a normal credential refresh is not possible.
///
/// # Arguments
/// * `client` - Signaling client for sending register request
/// * `register_request` - RegisterRequest containing actor type, realm, and service spec
/// * `credential_state` - Shared credential state to update
async fn re_register_task(
    client: Arc<dyn SignalingClient>,
    register_request: RegisterRequest,
    credential_state: CredentialState,
) {
    tracing::info!(
        "🔄 Re-registering actor after credential expiry (type: {}/{})",
        register_request.actr_type.manufacturer,
        register_request.actr_type.name
    );

    match client.send_register_request(register_request.clone()).await {
        Ok(register_response) => {
            match register_response.result {
                Some(actr_protocol::register_response::Result::Success(register_ok)) => {
                    let new_actor_id = &register_ok.actr_id;
                    let new_credential = register_ok.credential;
                    let new_expires_at = register_ok.credential_expires_at;
                    let new_psk = register_ok.psk;

                    // Update shared credential state
                    credential_state
                        .update(new_credential.clone(), new_expires_at, new_psk.clone())
                        .await;

                    tracing::info!(
                        "✅ Re-registration successful (ActrId: {}, new key_id: {})",
                        new_actor_id.to_string_repr(),
                        new_credential.token_key_id
                    );

                    if new_psk.is_some() {
                        tracing::debug!("🔑 PSK updated for TURN authentication");
                    }

                    if let Some(expires_at) = &new_expires_at {
                        tracing::debug!("⏰ New credential expires at: {}s", expires_at.seconds);
                    }
                }
                Some(actr_protocol::register_response::Result::Error(err)) => {
                    tracing::error!(
                        "❌ Re-registration failed: code={}, message={}",
                        err.code,
                        err.message
                    );
                }
                None => {
                    tracing::error!("❌ Re-registration response missing result");
                }
            }
        }
        Err(e) => {
            tracing::error!("❌ Failed to send re-register request: {}", e);
        }
    }
}
