//! Heartbeat management for ActrNode
//!
//! This module contains functions for sending periodic heartbeat messages
//! to the signaling server and handling responses.

use crate::lifecycle::CredentialState;
use crate::transport::error::NetworkError;
use crate::wire::webrtc::SignalingClient;
use actr_protocol::{ActrId, ActrIdExt, RegisterRequest, ServiceAvailabilityState};
use actr_runtime_mailbox::Mailbox;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use url::Url;

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
/// * `ais_base_url` - AIS base URL for HTTP registration
/// * `realm_secret` - Optional realm secret for AIS registration
///
/// Returns `Some(new_actor_id)` when re-registration assigns a new ActrId,
/// so the caller can update its loop variable for subsequent heartbeats.
async fn send_heartbeat_and_handle_response(
    client: &Arc<dyn SignalingClient>,
    actor_id: &ActrId,
    credential_state: &CredentialState,
    mailbox: &Arc<dyn Mailbox>,
    heartbeat_interval: Duration,
    register_request: &RegisterRequest,
    ais_base_url: &Url,
    realm_secret: Option<&str>,
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
            let new_actor_id = re_register_task(
                client.clone(),
                actor_id.clone(),
                register_request.clone(),
                credential_state.clone(),
                ais_base_url.clone(),
                realm_secret.map(str::to_owned),
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

        // Trigger immediate credential refresh via AIS HTTP in a spawned task
        let ais_url = ais_base_url.clone();
        let secret = realm_secret.map(str::to_owned);
        let req = register_request.clone();
        tokio::spawn(credential_refresh_task(
            client.clone(),
            actor_id.clone(),
            credential_state.clone(),
            ais_url,
            secret,
            req,
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
/// * `ais_base_url` - AIS base URL for HTTP registration
/// * `realm_secret` - Optional realm secret for AIS registration
pub async fn heartbeat_task(
    shutdown: CancellationToken,
    client: Arc<dyn SignalingClient>,
    actor_id: ActrId,
    credential_state: CredentialState,
    mailbox: Arc<dyn Mailbox>,
    heartbeat_interval: Duration,
    register_request: RegisterRequest,
    ais_base_url: Url,
    realm_secret: Option<String>,
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
                    &ais_base_url,
                    realm_secret.as_deref(),
                )
                .await {
                    tracing::info!(
                        "🔄 Heartbeat actor_id updated: {} -> {}",
                        actor_id.to_string_repr(),
                        new_actor_id.to_string_repr(),
                    );
                    actor_id = new_actor_id;
                }
            }
        }
    }
    tracing::info!("✅ Heartbeat task terminated gracefully");
}

/// Refresh credential for an actor via AIS HTTP endpoint
///
/// This function re-registers via AIS HTTP to obtain a fresh credential.
/// The signaling WebSocket connection remains intact (credential is still valid,
/// just nearing expiry).
///
/// # Arguments
/// * `client` - Signaling client for updating credential state
/// * `actor_id` - Actor ID for the credential update
/// * `credential_state` - Shared credential state to update
/// * `ais_base_url` - AIS base URL for HTTP registration
/// * `realm_secret` - Optional realm secret
/// * `register_request` - RegisterRequest for re-registration
async fn credential_refresh_task(
    client: Arc<dyn SignalingClient>,
    actor_id: ActrId,
    credential_state: CredentialState,
    ais_base_url: Url,
    realm_secret: Option<String>,
    register_request: RegisterRequest,
) {
    tracing::info!(
        "🔑 Refreshing credential for Actor {} via AIS HTTP",
        actor_id.to_string_repr()
    );

    match crate::ais_registration::register_with_ais(
        &ais_base_url,
        &register_request,
        realm_secret.as_deref(),
    )
    .await
    {
        Ok(register_ok) => {
            let new_credential = register_ok.credential;
            let new_expires_at = register_ok.credential_expires_at;
            let new_turn_credential = Some(register_ok.turn_credential);

            // Update shared credential state
            credential_state
                .update(new_credential.clone(), new_expires_at, new_turn_credential)
                .await;

            // Update signaling client credential for reconnect URL
            client.set_credential_state(credential_state.clone()).await;

            tracing::info!(
                "✅ Credential refreshed successfully for Actor {}",
                actor_id.to_string_repr(),
            );

            if let Some(expires_at) = &new_expires_at {
                tracing::debug!("⏰ New credential expires at: {}s", expires_at.seconds);
            }
        }
        Err(e) => {
            tracing::warn!("⚠️ Credential refresh via AIS failed: {}", e);
        }
    }
}

/// Re-register actor after credential expiry via AIS HTTP
///
/// When the credential has completely expired, the solution is:
/// 1. Register via AIS HTTP to get fresh credentials
/// 2. Disconnect old signaling WebSocket (server cleans up stale state)
/// 3. Reconnect signaling WebSocket with new identity in URL
///
/// # Arguments
/// * `client` - Signaling client for WebSocket management
/// * `actor_id` - Current actor ID (used for logging)
/// * `register_request` - RegisterRequest for re-registration
/// * `credential_state` - Shared credential state to update
/// * `ais_base_url` - AIS base URL for HTTP registration
/// * `realm_secret` - Optional realm secret
async fn re_register_task(
    client: Arc<dyn SignalingClient>,
    actor_id: ActrId,
    register_request: RegisterRequest,
    credential_state: CredentialState,
    ais_base_url: Url,
    realm_secret: Option<String>,
) -> ActrId {
    tracing::info!(
        "🔄 Re-registering actor {} after credential expiry via AIS HTTP (type: {}/{})",
        actor_id.to_string_repr(),
        register_request.actr_type.manufacturer,
        register_request.actr_type.name
    );

    // Step 1: Register via AIS HTTP to get fresh credentials
    let register_ok = match crate::ais_registration::register_with_ais(
        &ais_base_url,
        &register_request,
        realm_secret.as_deref(),
    )
    .await
    {
        Ok(ok) => ok,
        Err(e) => {
            tracing::error!("❌ Re-registration via AIS failed: {}", e);
            return actor_id;
        }
    };

    let new_actor_id = register_ok.actr_id.clone();
    let new_credential = register_ok.credential;
    let new_expires_at = register_ok.credential_expires_at;
    let new_turn_credential = Some(register_ok.turn_credential);

    // Step 2: Update shared credential state
    credential_state
        .update(new_credential.clone(), new_expires_at, new_turn_credential)
        .await;

    // Step 3: Disconnect old signaling WebSocket
    tracing::info!("🔌 Disconnecting signaling client to clear stale registration");
    client.clear_identity().await;
    if let Err(e) = client.disconnect().await {
        tracing::warn!("⚠️ Disconnect failed (non-fatal, continuing): {}", e);
    }

    // Step 4: Set new identity and reconnect
    client.set_actor_id(new_actor_id.clone()).await;
    client.set_credential_state(credential_state.clone()).await;

    tracing::info!("🔗 Reconnecting signaling client with new credentials");
    if let Err(e) = client.connect().await {
        tracing::error!("❌ Failed to reconnect for re-registration: {}", e);
        return new_actor_id;
    }

    tracing::info!(
        "✅ Re-registration successful (ActrId: {})",
        new_actor_id.to_string_repr(),
    );

    if let Some(expires_at) = &new_expires_at {
        tracing::debug!("⏰ New credential expires at: {}s", expires_at.seconds);
    }

    new_actor_id
}
