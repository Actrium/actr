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
//! 4. On 401 or locally-expired renewal token: hard rebind via `/register`.
//! 5. On 403: transition to `RealmUnavailable`, stop retrying.
//! 6. Temporary errors: exponential backoff with jitter.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use prost::bytes::Bytes;
use tokio::sync::Mutex;

use super::session_state::SessionState;

// ---- Registration Context --------------------------------------------------

/// Saved registration parameters so hard rebind can call `/register` again
/// with the same authentication context.
#[derive(Clone)]
pub(crate) enum RegistrationContext {
    /// Package-backed registration — carries the full original request
    /// including manifest bytes and MFR signature.
    Package {
        #[allow(dead_code)]
        request: actr_protocol::RegisterRequest,
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

// ---- Renewal result types --------------------------------------------------

/// Result of a soft renew (POST /ais/renew).
pub(crate) struct SoftRenewResult {
    pub credential: actr_protocol::AIdCredential,
    pub credential_expires_at: actr_protocol::prost_types::Timestamp,
    pub turn_credential: actr_protocol::TurnCredential,
    pub renewal_token: Bytes,
    pub renewal_token_expires_at: actr_protocol::prost_types::Timestamp,
}

/// Result of a hard rebind (POST /register).
pub(crate) struct HardRebindResult {
    pub actr_id: actr_protocol::ActrId,
    pub credential: actr_protocol::AIdCredential,
    pub credential_expires_at: actr_protocol::prost_types::Timestamp,
    pub turn_credential: actr_protocol::TurnCredential,
    pub renewal_token: Bytes,
    pub renewal_token_expires_at: actr_protocol::prost_types::Timestamp,
}

// ---- Credential Manager ----------------------------------------------------

/// Shared credential manager — clonable, all clones share the same state.
pub(crate) struct CredentialManager {
    session: SessionState,
    registration_ctx: RegistrationContext,

    /// Single-flight guard: only one renewal attempt at a time.
    renewing: Arc<AtomicBool>,
    /// Pending renewal join handle for cancellation during shutdown.
    inflight: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl CredentialManager {
    pub(crate) fn new(session: SessionState, registration_ctx: RegistrationContext) -> Self {
        Self {
            session,
            registration_ctx,
            renewing: Arc::new(AtomicBool::new(false)),
            inflight: Arc::new(Mutex::new(None)),
        }
    }

    /// Return a clone of the managed SessionState.
    pub(crate) fn session_state(&self) -> SessionState {
        self.session.clone()
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

        // Spawn the actual work so the caller isn't blocked.
        let handle = tokio::spawn(async move {
            let backoff = Backoff::new();
            // TODO(Section 5): implement actual renewal loop calling
            // AisClient::renew_credential() and atomically updating
            // SessionState via update_credentials() or commit_hard_rebind().
            //
            // See PLAN.md §5.3 for the full state machine:
            // - Soft renew → update_credentials()
            // - 401 / expired → hard rebind → commit_hard_rebind()
            // - 403 → set_realm_unavailable()
            // - Retryable → backoff

            let _ = backoff;
            let _ = session;

            tracing::info!(
                "CredentialManager: renewal loop placeholder — to be wired to AisClient::renew_credential()"
            );
        });

        // Store the handle for potential cancellation during shutdown.
        let inflight = self.inflight.clone();
        tokio::spawn(async move {
            let mut guard = inflight.lock().await;
            *guard = Some(handle);
        });
    }

    /// Cancel any in-flight renewal (called during shutdown).
    pub(crate) async fn cancel(&self) {
        let mut guard = self.inflight.lock().await;
        if let Some(handle) = guard.take() {
            handle.abort();
        }
        self.renewing.store(false, Ordering::Release);
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
mod tests {
    use super::*;

    #[test]
    fn backoff_sequence() {
        let mut b = Backoff::new();
        let d0 = b.next();
        let d1 = b.next();
        let d2 = b.next();
        let d3 = b.next();
        let d4 = b.next();

        assert!(d0 >= Duration::from_secs(1));
        assert!(d1 >= Duration::from_secs(1));
        assert!(d2 >= Duration::from_secs(1));
        assert!(d3 >= Duration::from_secs(1));
        assert!(d4 <= Duration::from_secs(75)); // 60 + 25% jitter
    }
}
