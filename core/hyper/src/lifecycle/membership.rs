//! Membership authority — the single owner of the node credential.
//!
//! A signaling handshake can reject a credential before a heartbeat connection
//! exists. The membership controller gives that socket path an on-demand route
//! to the credential manager without turning heartbeat into a second socket
//! driver or adding a background refresh scheduler.
//!
//! The controller owns the credential publication channel. Each successful
//! credential change advances a publication revision, including soft renewals;
//! this revision is deliberately separate from the session generation used to
//! invalidate runtime contexts after a hard rebind.

use std::sync::Arc;

use actr_protocol::{AIdCredential, ActrId, TurnCredential};
use prost_types::Timestamp;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::transport::AuthVerdict;

use super::credential_manager::{CredentialManager, ReacquireOutcome};
use super::session_state::SessionPhase;

/// Credential material published to socket consumers.
#[derive(Clone, Debug)]
pub struct PublishedCredential {
    pub credential: AIdCredential,
    pub credential_expires_at: Option<Timestamp>,
    pub turn_credential: Option<TurnCredential>,
    pub actor_id: ActrId,
    /// Monotonic publication revision. It advances on every successful
    /// credential change and is never compared with session or reconnect
    /// generations.
    pub revision: u64,
}

/// Result of one on-demand membership request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MembershipResolution {
    /// This request published a fresh credential.
    Published,
    /// A prior request already advanced beyond the reported revision.
    Superseded,
    /// AIS was temporarily unavailable. The socket may apply its normal
    /// reconnect backoff; a later auth rejection is a new on-demand trigger.
    Deferred,
    /// The realm denied membership. No automatic re-probe is scheduled.
    Denied,
    /// The controller shut down before resolving the request.
    Shutdown,
}

struct MembershipReport {
    verdict: AuthVerdict,
    stale_revision: u64,
    resolution_tx: Option<oneshot::Sender<MembershipResolution>>,
}

impl MembershipReport {
    fn untracked(verdict: AuthVerdict, stale_revision: u64) -> Self {
        Self {
            verdict,
            stale_revision,
            resolution_tx: None,
        }
    }

    fn tracked(
        verdict: AuthVerdict,
        stale_revision: u64,
    ) -> (Self, oneshot::Receiver<MembershipResolution>) {
        let (resolution_tx, resolution_rx) = oneshot::channel();
        (
            Self {
                verdict,
                stale_revision,
                resolution_tx: Some(resolution_tx),
            },
            resolution_rx,
        )
    }

    fn finish(mut self, resolution: MembershipResolution) {
        if let Some(tx) = self.resolution_tx.take() {
            let _ = tx.send(resolution);
        }
    }
}

/// Cheap, clonable handle used by signaling and heartbeat reporters.
#[derive(Clone)]
pub struct MembershipHandle {
    credential_rx: watch::Receiver<Arc<PublishedCredential>>,
    report_tx: mpsc::Sender<MembershipReport>,
}

impl MembershipHandle {
    pub fn credential_rx(&self) -> watch::Receiver<Arc<PublishedCredential>> {
        self.credential_rx.clone()
    }

    pub fn current_revision(&self) -> u64 {
        self.credential_rx.borrow().revision
    }

    /// Submit an on-demand trigger without waiting for its outcome. Heartbeat
    /// uses this path because it never drives or parks the socket.
    pub async fn report(&self, verdict: AuthVerdict, stale_revision: u64) {
        if let Err(error) = self
            .report_tx
            .send(MembershipReport::untracked(verdict, stale_revision))
            .await
        {
            tracing::debug!(%error, "membership report dropped: controller gone");
        }
    }

    /// Submit an on-demand trigger and wait until the controller has either
    /// published, deferred, denied, or shut down. Signaling uses the result to
    /// avoid parking forever after a transient AIS failure.
    pub(crate) async fn resolve(
        &self,
        verdict: AuthVerdict,
        stale_revision: u64,
    ) -> MembershipResolution {
        let (report, resolution_rx) = MembershipReport::tracked(verdict, stale_revision);
        if self.report_tx.send(report).await.is_err() {
            return MembershipResolution::Shutdown;
        }
        resolution_rx
            .await
            .unwrap_or(MembershipResolution::Shutdown)
    }
}

/// Owns the credential and serializes on-demand re-acquire requests.
pub struct MembershipController {
    engine: CredentialManager,
    credential_tx: watch::Sender<Arc<PublishedCredential>>,
    report_rx: mpsc::Receiver<MembershipReport>,
    /// Publish happens-before this socket wake.
    wake_socket: Arc<dyn Fn() + Send + Sync>,
    shutdown: CancellationToken,
}

impl MembershipController {
    pub fn new(
        engine: CredentialManager,
        seed: PublishedCredential,
        wake_socket: Arc<dyn Fn() + Send + Sync>,
        shutdown: CancellationToken,
    ) -> (Self, MembershipHandle) {
        let (credential_tx, credential_rx) = watch::channel(Arc::new(seed));
        let (report_tx, report_rx) = mpsc::channel(64);
        (
            Self {
                engine,
                credential_tx,
                report_rx,
                wake_socket,
                shutdown,
            },
            MembershipHandle {
                credential_rx,
                report_tx,
            },
        )
    }

    fn current_revision(&self) -> u64 {
        self.credential_tx.borrow().revision
    }

    /// Stamp the next publication revision, publish the material, then wake the
    /// sole socket driver. The credential manager's session generation is not
    /// used as the publication clock.
    fn publish_then_wake(&self, mut published: PublishedCredential) {
        published.revision = self
            .current_revision()
            .checked_add(1)
            .expect("credential publication revision exhausted");
        let revision = published.revision;
        self.credential_tx.send_replace(Arc::new(published));
        (self.wake_socket)();
        tracing::info!(revision, "membership: published credential and woke socket");
    }

    pub async fn run(mut self) {
        tracing::info!("membership controller loop started");
        loop {
            let report = tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => {
                    tracing::info!("membership controller loop shutting down");
                    return;
                }
                report = self.report_rx.recv() => match report {
                    Some(report) => report,
                    None => return,
                }
            };

            let current = self.current_revision();
            if report.stale_revision < current {
                tracing::debug!(
                    stale_revision = report.stale_revision,
                    current_revision = current,
                    "membership: request already handled"
                );
                report.finish(MembershipResolution::Superseded);
                continue;
            }

            let resolution = match report.verdict {
                AuthVerdict::RealmDenied => {
                    self.mark_denied().await;
                    MembershipResolution::Denied
                }
                AuthVerdict::Rejected => self.handle_rejection(report.stale_revision).await,
            };
            report.finish(resolution);
        }
    }

    async fn handle_rejection(&mut self, stale_revision: u64) -> MembershipResolution {
        if self.engine.phase().await == SessionPhase::RealmUnavailable {
            return MembershipResolution::Denied;
        }

        tracing::warn!(
            stale_revision,
            "membership: credential rejected; running one on-demand re-acquire"
        );
        match self.engine.reacquire().await {
            ReacquireOutcome::Renewed(published) => {
                self.publish_then_wake(published);
                MembershipResolution::Published
            }
            ReacquireOutcome::Denied => {
                self.mark_denied().await;
                MembershipResolution::Denied
            }
            ReacquireOutcome::Deferred(reason) => {
                tracing::warn!(
                    %reason,
                    "membership: on-demand re-acquire deferred; awaiting another external trigger"
                );
                MembershipResolution::Deferred
            }
        }
    }

    async fn mark_denied(&self) {
        self.engine.mark_realm_unavailable().await;
        tracing::error!(
            error_category = "membership_denied",
            severity = 10,
            "membership: realm denied; automatic recovery is stopped"
        );
    }
}

/// Spawn only the on-demand coordinator. Credential expiry does not create a
/// background refresh schedule; heartbeat/signaling warnings remain the trigger.
pub fn spawn_membership_controller(
    controller: MembershipController,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(controller.run())
}

#[cfg(test)]
#[path = "membership_tests.rs"]
mod tests;
