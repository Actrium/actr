//! Credential Manager — single-flight soft renew / hard rebind orchestration.
//!
//! # Trigger sources
//!
//! - Access credential expiry scheduler (5 min before expiry + 0–30s jitter).
//! - Heartbeat / signaling returns 401.
//! - (Legacy) signaling credential warning.
//!
//! # Behaviour
//!
//! 1. All triggers enter the same single-flight future.
//! 2. Call `POST /ais/renew`.
//! 3. On success: atomically replace credentials (soft renew).
//! 4. On 401 or locally-expired renewal token: stable-identity `/reissue`.
//! 5. On 403: transition to `RealmUnavailable`, stop retrying.
//! 6. Temporary errors: exponential backoff with jitter.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actr_protocol::prost::Message as _;
use actr_protocol::{
    IdentityClaims, ReissueCredentialRequest, RenewCredentialRequest, register_response,
    renew_credential_response,
};
use tokio::sync::Mutex;

use crate::ais_client::{AisClient, RenewError};
use crate::transport::PeerTransport;
use crate::wire::webrtc::gate::WebRtcGate;
use crate::wire::webrtc::{HookCallback, HookEvent, SignalingClient, WebRtcCoordinator};

use super::membership::PublishedCredential;
use super::node::CredentialState;
use super::session_state::{SessionPhase, SessionSnapshot, SessionState};

// ---- Re-acquire outcome (membership controller path) -----------------------

/// Terminal outcome of a single-flight re-acquire driven by the membership
/// controller. Unlike the legacy fire-and-forget `trigger_renewal`, the
/// controller path is awaitable and returns exactly what happened so the
/// coordinator can publish, back off, or enter the terminal denied phase.
// Transient outcome: constructed once per re-acquire and matched immediately,
// never stored in bulk, so the large `Renewed` variant is not worth boxing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum ReacquireOutcome {
    /// A fresh credential was minted (soft renew or credential-only hard
    /// rebind). Carries the generation-stamped credential to publish.
    Renewed(PublishedCredential),
    /// The realm denied membership (403 / `RealmUnavailable`). Terminal for
    /// automatic recovery.
    Denied,
    /// A transient failure — retry later on the backoff ladder.
    Deferred(String),
}

// ---- Registration Context --------------------------------------------------

/// Saved registration parameters so hard rebind can authenticate `/reissue`
/// with the same package or linked-workload context.
#[derive(Clone)]
pub(crate) enum RegistrationContext {
    /// Package-backed registration — carries the full original request
    /// including manifest bytes and MFR signature.
    ///
    /// `resign` is the manufacturer re-signing capability. It is `Some` when the
    /// initial registration carried a manufacturer proof (unpublished package). Hard
    /// rebind re-invokes it to mint a fresh proof — the original nonce was
    /// consumed by AIS on first success and cannot be reused.
    Package {
        #[allow(dead_code)]
        request: actr_protocol::RegisterRequest,
        resign: Option<Arc<dyn crate::ManufacturerAuthProvider>>,
    },
    /// Source-linked registration — carries the request and an optional
    /// realm secret (kept in memory only, never logged).
    Linked {
        #[allow(dead_code)]
        request: actr_protocol::RegisterRequest,
        #[allow(dead_code)]
        realm_secret: Option<String>,
    },
}

// ---- Credential Manager ----------------------------------------------------

/// Shared credential manager — clonable, all clones share the same state.
#[derive(Clone)]
pub(crate) struct CredentialManager {
    session: SessionState,
    registration_ctx: RegistrationContext,
    ais_endpoint: String,
    realm_secret: Option<String>,

    /// Single-flight guard: only one renewal attempt at a time.
    renewing: Arc<AtomicBool>,
    /// Pending renewal join handle for cancellation during shutdown.
    inflight: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Runtime handles that must be updated after hard rebind commits.
    hard_rebind_handles: Arc<Mutex<Option<HardRebindHandles>>>,
    /// Lifecycle callback notified after soft renewal commits.
    hook_callback: Arc<Mutex<Option<HookCallback>>>,
}

#[derive(Clone)]
pub(crate) struct HardRebindHandles {
    pub signaling_client: Arc<dyn SignalingClient>,
    pub credential_state: CredentialState,
    pub webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
    pub webrtc_gate: Option<Arc<WebRtcGate>>,
    pub peer_transport: Option<Arc<PeerTransport>>,
}

impl CredentialManager {
    pub(crate) fn new(
        session: SessionState,
        registration_ctx: RegistrationContext,
        ais_endpoint: impl Into<String>,
        realm_secret: Option<String>,
    ) -> Self {
        Self {
            session,
            registration_ctx,
            ais_endpoint: ais_endpoint.into(),
            realm_secret,
            renewing: Arc::new(AtomicBool::new(false)),
            inflight: Arc::new(Mutex::new(None)),
            hard_rebind_handles: Arc::new(Mutex::new(None)),
            hook_callback: Arc::new(Mutex::new(None)),
        }
    }

    /// Return a clone of the managed SessionState.
    pub(crate) fn session_state(&self) -> SessionState {
        self.session.clone()
    }

    pub(crate) async fn install_hard_rebind_handles(&self, handles: HardRebindHandles) {
        *self.hard_rebind_handles.lock().await = Some(handles);
    }

    pub(crate) async fn install_hook_callback(&self, callback: Option<HookCallback>) {
        *self.hook_callback.lock().await = callback;
    }

    // ---- Membership-controller path (credential-only) ----------------------

    /// Current session phase.
    pub(crate) async fn phase(&self) -> SessionPhase {
        self.session.phase().await
    }

    /// Transition the session to `RealmUnavailable` (terminal denied).
    pub(crate) async fn mark_realm_unavailable(&self) {
        self.session.set_realm_unavailable().await;
    }

    /// Snapshot the current credential as a generation-stamped
    /// [`PublishedCredential`] for the membership `watch` channel.
    pub(crate) async fn published_credential(&self) -> PublishedCredential {
        let snapshot = self.session.snapshot().await;
        PublishedCredential {
            credential: snapshot.credential,
            credential_expires_at: Some(snapshot.credential_expires_at),
            turn_credential: Some(snapshot.turn_credential),
            actor_id: snapshot.actor_id,
            // Seed the publication clock. Subsequent results are stamped by
            // MembershipController rather than copied from session generation.
            revision: snapshot.generation,
        }
    }

    /// Single-flight, credential-only re-acquire driven by the membership
    /// controller.
    ///
    /// Mints a fresh credential (soft renew, then hard `/reissue` fallback) and
    /// updates identity/handles, but NEVER drives the socket — the signaling
    /// client is the sole reconnect driver. The coordinator publishes the
    /// returned credential and wakes the socket itself.
    ///
    /// Runtime hard rebind is credential-only against the stable on-disk node
    /// AID. AIS authenticates the saved registration context but reissues for
    /// the explicitly supplied existing identity.
    pub(crate) async fn reacquire(&self) -> ReacquireOutcome {
        let handles = self.hard_rebind_handles.lock().await.clone();
        let hook_callback = self.hook_callback.lock().await.clone();
        run_reacquire_once(
            self.session.clone(),
            self.ais_endpoint.clone(),
            self.realm_secret.clone(),
            self.registration_ctx.clone(),
            handles,
            hook_callback,
        )
        .await
    }

    /// Entry point for all renewal triggers. Returns immediately if a
    /// renewal is already in flight (single-flight).
    pub(crate) fn trigger_renewal(&self) {
        // Fast-path: if already renewing, skip.
        if self.renewing.swap(true, Ordering::AcqRel) {
            tracing::debug!("CredentialManager: renewal already in flight, skipping trigger");
            return;
        }

        let session = self.session.clone();
        let ais_endpoint = self.ais_endpoint.clone();
        let realm_secret = self.realm_secret.clone();
        let registration_ctx = self.registration_ctx.clone();
        let renewing = self.renewing.clone();
        let hard_rebind_handles = self.hard_rebind_handles.clone();
        let hook_callback = self.hook_callback.clone();

        // Spawn the actual work so the caller isn't blocked.
        let handle = tokio::spawn(async move {
            let handles = hard_rebind_handles.lock().await.clone();
            let hook_callback = hook_callback.lock().await.clone();
            let result = run_renewal_once(
                session,
                ais_endpoint,
                realm_secret,
                registration_ctx,
                handles,
                hook_callback,
            )
            .await;
            if let Err(err) = result {
                tracing::warn!(error = %err, "CredentialManager: renewal attempt ended");
            }
            renewing.store(false, Ordering::Release);
        });

        // Store the handle for potential cancellation during shutdown.
        let inflight = self.inflight.clone();
        tokio::spawn(async move {
            let mut guard = inflight.lock().await;
            *guard = Some(handle);
        });
    }

    /// Cancel any in-flight renewal (called during shutdown).
    #[allow(dead_code)]
    pub(crate) async fn cancel(&self) {
        let mut guard = self.inflight.lock().await;
        if let Some(handle) = guard.take() {
            handle.abort();
        }
        self.renewing.store(false, Ordering::Release);
    }
}

async fn run_renewal_once(
    session: SessionState,
    ais_endpoint: String,
    realm_secret: Option<String>,
    registration_ctx: RegistrationContext,
    hard_rebind_handles: Option<HardRebindHandles>,
    hook_callback: Option<HookCallback>,
) -> Result<(), String> {
    let snapshot = session.snapshot().await;

    if snapshot.renewal_token.is_empty() {
        return run_hard_rebind(
            session,
            ais_endpoint,
            realm_secret,
            registration_ctx,
            hard_rebind_handles,
        )
        .await;
    }

    if is_expired(snapshot.renewal_token_expires_at.seconds) {
        return run_hard_rebind(
            session,
            ais_endpoint,
            realm_secret,
            registration_ctx,
            hard_rebind_handles,
        )
        .await;
    }

    let mut ais = AisClient::new(&ais_endpoint);
    if let Some(secret) = realm_secret.as_deref() {
        ais = ais.with_realm_secret(secret);
    }

    let request = RenewCredentialRequest {
        actr_id: snapshot.actor_id.clone(),
        renewal_token: snapshot.renewal_token.clone(),
    };

    let response = match ais.renew_credential(request).await {
        Ok(response) => response,
        Err(RenewError::RealmUnavailable) => {
            session.set_realm_unavailable().await;
            return Err("realm unavailable during renewal".to_string());
        }
        Err(RenewError::TokenRejected) => {
            return run_hard_rebind(
                session,
                ais_endpoint,
                realm_secret,
                registration_ctx,
                hard_rebind_handles,
            )
            .await;
        }
        Err(RenewError::RateLimited { retry_after }) => {
            if let Some(delay) = retry_after {
                tokio::time::sleep(delay).await;
            }
            return Err("renewal rate limited".to_string());
        }
        Err(RenewError::Retryable(err)) => {
            let mut backoff = Backoff::new();
            tokio::time::sleep(backoff.next()).await;
            return Err(format!("retryable renew error: {err}"));
        }
        Err(err) => return Err(err.to_string()),
    };

    let ok = match response.result {
        Some(renew_credential_response::Result::Success(ok)) => ok,
        Some(renew_credential_response::Result::Error(err)) => {
            return Err(format!(
                "renew response contained error {}: {}",
                err.code, err.message
            ));
        }
        None => return Err("renew response missing result".to_string()),
    };

    if ok.actr_id != snapshot.actor_id {
        return Err("renew response changed ActrId".to_string());
    }

    let claims = IdentityClaims::decode(ok.credential.claims.as_ref())
        .map_err(|e| format!("renew credential claims decode failed: {e}"))?;
    if claims.actor_id != snapshot.actor_id.to_string_repr() {
        return Err("renew credential claims actor_id mismatch".to_string());
    }

    let credential_expires_at = ok
        .credential_expires_at
        .ok_or_else(|| "renew response missing credential expiry".to_string())?;
    let renewal_token = ok
        .renewal_token
        .ok_or_else(|| "renew response missing renewal token".to_string())?;
    let renewal_token_expires_at = ok
        .renewal_token_expires_at
        .ok_or_else(|| "renew response missing renewal token expiry".to_string())?;

    session
        .update_credentials(
            ok.credential.clone(),
            credential_expires_at,
            ok.turn_credential.clone(),
            renewal_token.clone(),
            renewal_token_expires_at,
        )
        .await;

    if let Some(handles) = hard_rebind_handles {
        handles
            .credential_state
            .update(
                ok.credential,
                Some(credential_expires_at),
                Some(ok.turn_credential),
            )
            .await;
    }

    fire_credential_renewed(hook_callback.as_ref(), &credential_expires_at).await;

    tracing::info!(
        actor_id = %snapshot.actor_id.to_string_repr(),
        credential_expires_at = credential_expires_at.seconds,
        renewal_token_expires_at = renewal_token_expires_at.seconds,
        "CredentialManager: soft renewal completed"
    );

    Ok(())
}

async fn run_hard_rebind(
    session: SessionState,
    ais_endpoint: String,
    realm_secret: Option<String>,
    registration_ctx: RegistrationContext,
    hard_rebind_handles: Option<HardRebindHandles>,
) -> Result<(), String> {
    let old_snapshot = session.snapshot().await;
    tracing::warn!(
        actor_id = %old_snapshot.actor_id.to_string_repr(),
        generation = old_snapshot.generation,
        "CredentialManager: starting hard rebind"
    );

    let ais = AisClient::new(&ais_endpoint);
    let (ais, registration) =
        build_reissue_registration(ais, registration_ctx, realm_secret).await?;

    let response = ais
        .reissue_credential(ReissueCredentialRequest {
            actr_id: old_snapshot.actor_id.clone(),
            registration,
            renewal_proof: reissue_possession_proof(&old_snapshot),
        })
        .await
        .map_err(|err| format!("hard rebind reissue failed before commit: {err}"))?;

    let ok = match response.result {
        Some(register_response::Result::Success(ok)) => ok,
        Some(register_response::Result::Error(err)) => {
            return Err(format!(
                "hard rebind reissue rejected before commit {}: {}",
                err.code, err.message
            ));
        }
        None => return Err("hard rebind reissue response missing result".to_string()),
    };

    if ok.actr_id != old_snapshot.actor_id {
        return Err(format!(
            "credential reissue changed ActrId ({} != {})",
            ok.actr_id.to_string_repr(),
            old_snapshot.actor_id.to_string_repr()
        ));
    }

    let credential_expires_at = ok
        .credential_expires_at
        .ok_or_else(|| "hard rebind response missing credential expiry".to_string())?;
    let renewal_token = ok
        .renewal_token
        .ok_or_else(|| "hard rebind response missing renewal token".to_string())?;
    let renewal_token_expires_at = ok
        .renewal_token_expires_at
        .ok_or_else(|| "hard rebind response missing renewal token expiry".to_string())?;

    let new_snapshot = SessionSnapshot {
        actor_id: old_snapshot.actor_id.clone(),
        credential: ok.credential.clone(),
        credential_expires_at,
        turn_credential: ok.turn_credential.clone(),
        renewal_token,
        renewal_token_expires_at,
        generation: old_snapshot.generation.saturating_add(1),
    };

    session.enter_rebinding().await;
    let _old = session.commit_hard_rebind(new_snapshot.clone()).await;

    if let Some(handles) = hard_rebind_handles {
        let _cleanup_guard = handles
            .webrtc_coordinator
            .as_ref()
            .map(|coordinator| coordinator.cleanup_guard());

        // Stop ingress on the old authenticated socket before draining peer
        // state. Otherwise a delayed old-identity Offer or RoleAssignment can
        // recreate a peer between close-all and disconnect.
        if let Err(err) = handles.signaling_client.disconnect().await {
            tracing::warn!(error = %err, "hard rebind signaling disconnect failed");
        }

        if let Some(coordinator) = handles.webrtc_coordinator.as_ref()
            && let Err(err) = coordinator.close_all_peers_immediately().await
        {
            tracing::warn!(error = %err, "hard rebind failed to close old WebRTC peers");
        }
        if let Some(peer_transport) = handles.peer_transport.as_ref()
            && let Err(err) = peer_transport.close_all().await
        {
            tracing::warn!(error = %err, "hard rebind failed to close old peer transports");
        }
        // Finalize after cancelling PeerTransport creators that may have
        // crossed the first coordinator drain.
        if let Some(coordinator) = handles.webrtc_coordinator.as_ref()
            && let Err(err) = coordinator.close_all_peers_immediately().await
        {
            tracing::warn!(error = %err, "hard rebind failed to finalize WebRTC peer cleanup");
        }

        handles
            .credential_state
            .update(
                new_snapshot.credential.clone(),
                Some(new_snapshot.credential_expires_at),
                Some(new_snapshot.turn_credential.clone()),
            )
            .await;

        handles
            .signaling_client
            .set_actor_id(new_snapshot.actor_id.clone())
            .await;
        handles
            .signaling_client
            .set_credential_state(handles.credential_state.clone())
            .await;

        if let Some(coordinator) = handles.webrtc_coordinator.as_ref() {
            coordinator
                .set_local_id(new_snapshot.actor_id.clone())
                .await;
        }
        if let Some(gate) = handles.webrtc_gate.as_ref() {
            gate.set_local_id(new_snapshot.actor_id.clone()).await;
        }
        match handles.signaling_client.connect_once().await {
            Ok(()) => session.set_active().await,
            Err(err) => {
                handles.signaling_client.schedule_auto_reconnect();
                return Err(format!(
                    "hard rebind committed but signaling reconnect failed: {err}"
                ));
            }
        }
    } else {
        session.set_active().await;
    }

    tracing::info!(
        actor_id = %new_snapshot.actor_id.to_string_repr(),
        generation = new_snapshot.generation,
        credential_expires_at = new_snapshot.credential_expires_at.seconds,
        renewal_token_expires_at = new_snapshot.renewal_token_expires_at.seconds,
        "CredentialManager: hard rebind committed"
    );

    Ok(())
}

// ---- Membership-controller re-acquire (credential-only) --------------------

/// Build the authenticated registration payload for `/reissue`, applying the manufacturer
/// re-sign for package registrations and installing the realm secret on `ais`.
///
/// Shared by the legacy `run_hard_rebind` semantics and the credential-only
/// controller path so the re-sign / replay-avoidance logic lives once.
async fn build_reissue_registration(
    mut ais: AisClient,
    registration_ctx: RegistrationContext,
    realm_secret: Option<String>,
) -> Result<(AisClient, actr_protocol::RegisterRequest), String> {
    let request = match registration_ctx {
        RegistrationContext::Package { request, resign } => {
            let mut request = request;
            if let Some(provider) = resign.as_ref() {
                let realm_id = request.realm.realm_id;
                let actr_type = request.actr_type.clone();
                let target = request
                    .target
                    .clone()
                    .filter(|target| !target.is_empty())
                    .ok_or_else(|| {
                        "hard rebind manufacturer re-sign failed: package target is missing"
                            .to_string()
                    })?;
                let manifest_raw = request
                    .manifest_raw
                    .as_ref()
                    .filter(|manifest| !manifest.is_empty())
                    .map(|manifest| manifest.to_vec())
                    .ok_or_else(|| {
                        "hard rebind manufacturer re-sign failed: package manifest is missing"
                            .to_string()
                    })?;
                let fresh = crate::sign_manufacturer_proof(
                    Arc::clone(provider),
                    realm_id,
                    actr_type,
                    target,
                    manifest_raw,
                )
                .await
                .map_err(|e| format!("hard rebind manufacturer re-sign failed: {e}"))?;
                request.manufacturer_auth_signature = Some(bytes::Bytes::from(fresh.signature));
                request.manufacturer_auth_signed_at = Some(fresh.signed_at);
                request.manufacturer_auth_nonce = Some(bytes::Bytes::from(fresh.nonce));
            }
            request
        }
        RegistrationContext::Linked {
            request,
            realm_secret,
        } => {
            if let Some(secret) = realm_secret {
                ais = ais.with_realm_secret(secret);
            }
            request
        }
    };
    if let Some(secret) = realm_secret {
        ais = ais.with_realm_secret(secret);
    }
    Ok((ais, request))
}

/// Credential-only single-flight re-acquire for the membership controller.
///
/// Tries a soft renew first (if the renewal token is usable), then falls back to
/// a stable-identity credential reissue. NEVER drives the socket. Returns the
/// generation-stamped credential to publish, or a terminal / deferred outcome.
async fn run_reacquire_once(
    session: SessionState,
    ais_endpoint: String,
    realm_secret: Option<String>,
    registration_ctx: RegistrationContext,
    hard_rebind_handles: Option<HardRebindHandles>,
    hook_callback: Option<HookCallback>,
) -> ReacquireOutcome {
    let snapshot = session.snapshot().await;

    let token_usable = !snapshot.renewal_token.is_empty()
        && !is_expired(snapshot.renewal_token_expires_at.seconds);

    if token_usable {
        match run_soft_renew(
            &session,
            &ais_endpoint,
            realm_secret.as_deref(),
            hard_rebind_handles.as_ref(),
            hook_callback.as_ref(),
            &snapshot,
        )
        .await
        {
            SoftRenewOutcome::Renewed(published) => return ReacquireOutcome::Renewed(published),
            SoftRenewOutcome::Denied => return ReacquireOutcome::Denied,
            SoftRenewOutcome::Deferred(reason) => return ReacquireOutcome::Deferred(reason),
            // Token was rejected by AIS — fall through to hard rebind.
            SoftRenewOutcome::FallThroughToHardRebind => {}
        }
    }

    run_credential_only_hard_rebind(
        session,
        ais_endpoint,
        realm_secret,
        registration_ctx,
        hard_rebind_handles,
    )
    .await
}

// Transient outcome (see `ReacquireOutcome`): local, immediately matched.
#[allow(clippy::large_enum_variant)]
enum SoftRenewOutcome {
    Renewed(PublishedCredential),
    Denied,
    Deferred(String),
    FallThroughToHardRebind,
}

/// One soft-renew round-trip (`POST /renew`). Credential-only: updates session +
/// credential_state, fires the renewed hook, and returns the published
/// credential. Never touches the socket.
async fn run_soft_renew(
    session: &SessionState,
    ais_endpoint: &str,
    realm_secret: Option<&str>,
    hard_rebind_handles: Option<&HardRebindHandles>,
    hook_callback: Option<&HookCallback>,
    snapshot: &SessionSnapshot,
) -> SoftRenewOutcome {
    let mut ais = AisClient::new(ais_endpoint);
    if let Some(secret) = realm_secret {
        ais = ais.with_realm_secret(secret);
    }

    let request = RenewCredentialRequest {
        actr_id: snapshot.actor_id.clone(),
        renewal_token: snapshot.renewal_token.clone(),
    };

    let response = match ais.renew_credential(request).await {
        Ok(response) => response,
        Err(RenewError::RealmUnavailable) => {
            session.set_realm_unavailable().await;
            return SoftRenewOutcome::Denied;
        }
        Err(RenewError::TokenRejected) => return SoftRenewOutcome::FallThroughToHardRebind,
        Err(RenewError::RateLimited { .. }) => {
            return SoftRenewOutcome::Deferred("renewal rate limited".to_string());
        }
        Err(RenewError::Retryable(err)) => {
            return SoftRenewOutcome::Deferred(format!("retryable renew error: {err}"));
        }
        Err(err) => return SoftRenewOutcome::Deferred(err.to_string()),
    };

    let ok = match response.result {
        Some(renew_credential_response::Result::Success(ok)) => ok,
        Some(renew_credential_response::Result::Error(err)) => {
            return SoftRenewOutcome::Deferred(format!(
                "renew response contained error {}: {}",
                err.code, err.message
            ));
        }
        None => return SoftRenewOutcome::Deferred("renew response missing result".to_string()),
    };

    if ok.actr_id != snapshot.actor_id {
        return SoftRenewOutcome::Deferred("renew response changed ActrId".to_string());
    }

    let claims = match IdentityClaims::decode(ok.credential.claims.as_ref()) {
        Ok(claims) => claims,
        Err(e) => {
            return SoftRenewOutcome::Deferred(format!(
                "renew credential claims decode failed: {e}"
            ));
        }
    };
    if claims.actor_id != snapshot.actor_id.to_string_repr() {
        return SoftRenewOutcome::Deferred("renew credential claims actor_id mismatch".to_string());
    }

    let credential_expires_at = match ok.credential_expires_at {
        Some(v) => v,
        None => {
            return SoftRenewOutcome::Deferred("renew response missing credential expiry".into());
        }
    };
    let renewal_token = match ok.renewal_token.clone() {
        Some(v) => v,
        None => return SoftRenewOutcome::Deferred("renew response missing renewal token".into()),
    };
    let renewal_token_expires_at = match ok.renewal_token_expires_at {
        Some(v) => v,
        None => {
            return SoftRenewOutcome::Deferred(
                "renew response missing renewal token expiry".into(),
            );
        }
    };

    session
        .update_credentials(
            ok.credential.clone(),
            credential_expires_at,
            ok.turn_credential.clone(),
            renewal_token,
            renewal_token_expires_at,
        )
        .await;

    if let Some(handles) = hard_rebind_handles {
        handles
            .credential_state
            .update(
                ok.credential.clone(),
                Some(credential_expires_at),
                Some(ok.turn_credential.clone()),
            )
            .await;
    }

    fire_credential_renewed(hook_callback, &credential_expires_at).await;

    let generation = session.generation().await;
    tracing::info!(
        actor_id = %snapshot.actor_id.to_string_repr(),
        generation,
        credential_expires_at = credential_expires_at.seconds,
        "membership: soft renewal completed (credential-only)"
    );

    SoftRenewOutcome::Renewed(PublishedCredential {
        credential: ok.credential,
        credential_expires_at: Some(credential_expires_at),
        turn_credential: Some(ok.turn_credential),
        actor_id: snapshot.actor_id.clone(),
        // MembershipController assigns current_revision + 1 at publish time.
        revision: 0,
    })
}

/// Credential-only hard rebind: `POST /reissue` against the STABLE node AID.
///
/// Re-authenticates the registration context, mints a fresh credential for the
/// explicitly supplied existing AID, commits the new
/// snapshot, re-ids the runtime handles (gate / coordinator / transport) and
/// updates `credential_state`. It deliberately DOES NOT drive the socket — no
/// `disconnect()`, no `connect_once()`, no `schedule_auto_reconnect()`. The
/// signaling client is the sole reconnect driver; the coordinator publishes the
/// returned credential and wakes it.
async fn run_credential_only_hard_rebind(
    session: SessionState,
    ais_endpoint: String,
    realm_secret: Option<String>,
    registration_ctx: RegistrationContext,
    hard_rebind_handles: Option<HardRebindHandles>,
) -> ReacquireOutcome {
    let old_snapshot = session.snapshot().await;
    tracing::warn!(
        actor_id = %old_snapshot.actor_id.to_string_repr(),
        generation = old_snapshot.generation,
        "membership: starting credential-only hard rebind"
    );

    let ais = AisClient::new(&ais_endpoint);
    let (ais, registration) =
        match build_reissue_registration(ais, registration_ctx, realm_secret).await {
            Ok(pair) => pair,
            Err(reason) => return ReacquireOutcome::Deferred(reason),
        };

    let response = match ais
        .reissue_credential(ReissueCredentialRequest {
            actr_id: old_snapshot.actor_id.clone(),
            registration,
            renewal_proof: reissue_possession_proof(&old_snapshot),
        })
        .await
    {
        Ok(response) => response,
        Err(err) => {
            return ReacquireOutcome::Deferred(format!("hard rebind reissue failed: {err}"));
        }
    };

    let ok = match response.result {
        Some(register_response::Result::Success(ok)) => ok,
        Some(register_response::Result::Error(err)) => {
            // 403 realm-denied surfaces here as a register error code.
            if err.code == 403 {
                session.set_realm_unavailable().await;
                return ReacquireOutcome::Denied;
            }
            return ReacquireOutcome::Deferred(format!(
                "hard rebind reissue rejected {}: {}",
                err.code, err.message
            ));
        }
        None => {
            return ReacquireOutcome::Deferred(
                "hard rebind reissue response missing result".to_string(),
            );
        }
    };

    // `/reissue` is contractually stable-identity. Keep the response check as a
    // protocol guard before committing any credential material.
    if ok.actr_id != old_snapshot.actor_id {
        return ReacquireOutcome::Deferred(format!(
            "credential reissue returned a different ActrId ({} != {})",
            ok.actr_id.to_string_repr(),
            old_snapshot.actor_id.to_string_repr()
        ));
    }

    let credential_expires_at = match ok.credential_expires_at {
        Some(v) => v,
        None => {
            return ReacquireOutcome::Deferred(
                "hard rebind response missing credential expiry".into(),
            );
        }
    };
    let renewal_token = match ok.renewal_token.clone() {
        Some(v) => v,
        None => {
            return ReacquireOutcome::Deferred("hard rebind response missing renewal token".into());
        }
    };
    let renewal_token_expires_at = match ok.renewal_token_expires_at {
        Some(v) => v,
        None => {
            return ReacquireOutcome::Deferred(
                "hard rebind response missing renewal token expiry".into(),
            );
        }
    };

    let new_snapshot = SessionSnapshot {
        actor_id: old_snapshot.actor_id.clone(),
        credential: ok.credential.clone(),
        credential_expires_at,
        turn_credential: ok.turn_credential.clone(),
        renewal_token,
        renewal_token_expires_at,
        generation: old_snapshot.generation.saturating_add(1),
    };

    session.enter_rebinding().await;
    let _old = session.commit_hard_rebind(new_snapshot.clone()).await;

    if let Some(handles) = hard_rebind_handles {
        handles
            .credential_state
            .update(
                new_snapshot.credential.clone(),
                Some(new_snapshot.credential_expires_at),
                Some(new_snapshot.turn_credential.clone()),
            )
            .await;

        // Re-id the runtime handles to the (unchanged) AID. This is a no-op for
        // identity but keeps the code path uniform and future-proof; crucially
        // it does NOT touch the socket.
        handles
            .signaling_client
            .set_actor_id(new_snapshot.actor_id.clone())
            .await;
        if let Some(coordinator) = handles.webrtc_coordinator.as_ref() {
            coordinator
                .set_local_id(new_snapshot.actor_id.clone())
                .await;
        }
        if let Some(gate) = handles.webrtc_gate.as_ref() {
            gate.set_local_id(new_snapshot.actor_id.clone()).await;
        }
    }

    // Back to Active — the socket driver will reconnect on the wake edge.
    session.set_active().await;

    tracing::info!(
        actor_id = %new_snapshot.actor_id.to_string_repr(),
        generation = new_snapshot.generation,
        credential_expires_at = new_snapshot.credential_expires_at.seconds,
        "membership: credential-only hard rebind committed"
    );

    ReacquireOutcome::Renewed(PublishedCredential {
        credential: new_snapshot.credential,
        credential_expires_at: Some(new_snapshot.credential_expires_at),
        turn_credential: Some(new_snapshot.turn_credential),
        actor_id: new_snapshot.actor_id,
        // MembershipController assigns current_revision + 1 at publish time.
        revision: 0,
    })
}

/// Possession proof for `/ais/reissue`: the last renewal token issued to this
/// node, even when it is already expired or was rejected by `/ais/renew`.
/// AIS matches its hash against the issuer-recorded binding for the actor so
/// only the holder of that token can reissue credentials for this serial.
fn reissue_possession_proof(snapshot: &SessionSnapshot) -> Option<bytes::Bytes> {
    if snapshot.renewal_token.is_empty() {
        tracing::warn!(
            actor_id = %snapshot.actor_id.to_string_repr(),
            "hard rebind has no renewal token to prove actor possession; AIS will reject the reissue"
        );
        return None;
    }
    Some(snapshot.renewal_token.clone())
}

fn is_expired(expires_at: i64) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    expires_at <= now
}

async fn fire_credential_renewed(
    hook_callback: Option<&HookCallback>,
    expires_at: &prost_types::Timestamp,
) {
    if let Some(callback) = hook_callback {
        let new_expiry =
            SystemTime::UNIX_EPOCH + Duration::from_secs(expires_at.seconds.max(0) as u64);
        callback(HookEvent::CredentialRenewed { new_expiry }).await;
    }
}

// ---- Exponential backoff with jitter ---------------------------------------

struct Backoff {
    attempt: u32,
}

impl Backoff {
    fn new() -> Self {
        Self { attempt: 0 }
    }

    /// Returns the next delay: 5, 10, 20, 40, 60, 60, ... seconds with
    /// ±25% jitter, capped at 60s.
    #[allow(dead_code)]
    fn next(&mut self) -> Duration {
        let base = match self.attempt {
            0 => 5,
            1 => 10,
            2 => 20,
            3 => 40,
            _ => 60,
        };
        self.attempt += 1;

        // Deterministic jitter: use attempt number as seed.
        let jitter =
            (base as f64 * 0.25 * ((self.attempt.wrapping_mul(7)) as f64 % 2.0 - 1.0)) as i64;
        let ms = ((base * 1000) as i64 + jitter * 1000i64).max(1000);
        Duration::from_millis(ms as u64)
    }
}

#[cfg(test)]
#[path = "credential_manager_tests.rs"]
mod tests;
