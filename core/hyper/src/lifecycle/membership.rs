//! Membership authority — the single owner of the node credential.
//!
//! # Why this exists
//!
//! A long-running node could not self-recover after the AIS server restarted or
//! rotated its signing key: the signaling handshake started returning 401 and
//! the reconnect loop retried the SAME dead credential forever. The root cause
//! was three disjoint subsystems:
//!
//! 1. Registration was a one-shot in `ActrNode::start()`.
//! 2. The signaling reconnect loop type-erased the handshake 401 into a generic
//!    transient error and just backed off.
//! 3. The recovery engine ([`CredentialManager`]) was only reachable from the
//!    heartbeat path, which needs an already-established connection — so a
//!    handshake 401 never reached it.
//!
//! # The model — one owner, a reporting connection, a typed verdict
//!
//! - [`MembershipController`] is the SOLE OWNER / writer of the credential and
//!   the only runtime caller of `/renew` and `/register`. It runs a coordinator
//!   loop consuming [`MembershipReport`]s.
//! - The signaling client is the SOLE DRIVER of the socket and a pure consumer:
//!   it reads the credential from a `watch` channel and, on a typed auth
//!   verdict, sends a report and parks on the watch until a newer generation is
//!   published.
//! - Two typed channels are the whole junction:
//!   - `watch<Arc<PublishedCredential>>` — data (generation-stamped credential).
//!   - `mpsc<MembershipReport>` — control (auth verdicts).
//!
//! Because the report channel is NOT gated behind a live socket, a handshake
//! 401 finally reaches the controller — the exact gap that caused the bug.
//!
//! # Storm guards
//!
//! - **Type distinction:** re-acquire is reachable only from a `Rejected`
//!   verdict; a transport blip is a different [`crate::transport::NetworkError`]
//!   variant that never produces a verdict and never reaches the controller.
//! - **Single-flight, generation-fenced:** concurrent triggers for the same
//!   stale generation coalesce onto the in-flight attempt; reports whose
//!   `stale_generation` is already behind the current generation are dropped as
//!   already-handled; sequential DISTINCT-generation denials advance the
//!   backoff ladder.
//! - **Edge-triggered park:** consumers wait on `watch::Receiver::wait_for`
//!   (zero CPU/IO, no sleep).
//! - **Fleet decorrelation:** proactive renew runs on a per-node jittered timer;
//!   the hard-`/register` backoff carries real randomized jitter; terminal
//!   `Denied` re-probes on a slow fixed cadence, never a tight loop.

use std::sync::Arc;
use std::time::Duration;

use actr_protocol::{AIdCredential, ActrId, TurnCredential};
use prost_types::Timestamp;
use rand::Rng;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::transport::AuthVerdict;

use super::credential_manager::{CredentialManager, ReacquireOutcome};
use super::session_state::SessionPhase;

// ---------------------------------------------------------------------------
// Published credential (the `watch` payload)
// ---------------------------------------------------------------------------

/// A generation-stamped credential snapshot published to consumers.
///
/// Consumers (the signaling client) read `credential` to build the handshake
/// URL and compare `generation` to decide whether a newer credential has landed
/// since they parked. `generation` mirrors [`super::session_state::SessionState`]
/// generation — the credential namespace only. It is orthogonal to the socket's
/// `reconnect_generation`; the two are never compared.
#[derive(Clone, Debug)]
pub struct PublishedCredential {
    pub credential: AIdCredential,
    pub credential_expires_at: Option<Timestamp>,
    pub turn_credential: Option<TurnCredential>,
    pub actor_id: ActrId,
    pub generation: u64,
}

/// A control-plane report from the socket-driving consumer to the credential
/// owner. Sent when the connection observed a typed authentication verdict.
#[derive(Clone, Copy, Debug)]
pub struct MembershipReport {
    /// The typed verdict the transport surfaced (401 -> `Rejected`, 403 ->
    /// `RealmDenied`).
    pub verdict: AuthVerdict,
    /// The credential generation that produced this verdict. Reports whose
    /// `stale_generation` is already behind the current generation are dropped
    /// (already handled). Concurrent reports for the same stale generation
    /// coalesce onto one in-flight re-acquire.
    pub stale_generation: u64,
}

// ---------------------------------------------------------------------------
// Handles
// ---------------------------------------------------------------------------

/// Cheap, clonable handle the rest of the runtime uses to talk to the
/// controller. Holds the read side of the data channel and the send side of the
/// control channel — the two typed channels are the whole junction.
#[derive(Clone)]
pub struct MembershipHandle {
    credential_rx: watch::Receiver<Arc<PublishedCredential>>,
    report_tx: mpsc::Sender<MembershipReport>,
}

impl MembershipHandle {
    /// A live receiver on the published-credential channel.
    pub fn credential_rx(&self) -> watch::Receiver<Arc<PublishedCredential>> {
        self.credential_rx.clone()
    }

    /// The report sender for consumers that observe an auth verdict.
    pub fn report_tx(&self) -> mpsc::Sender<MembershipReport> {
        self.report_tx.clone()
    }

    /// Fire-and-forget a report. Best-effort: if the controller has shut down
    /// the send fails silently — the socket path must not block on it.
    pub async fn report(&self, report: MembershipReport) {
        if let Err(e) = self.report_tx.send(report).await {
            tracing::debug!(error = %e, "membership report dropped: controller gone");
        }
    }
}

// ---------------------------------------------------------------------------
// Backoff ladder (real randomized jitter)
// ---------------------------------------------------------------------------

/// Hard-`/register` backoff ladder with REAL randomized jitter.
///
/// The old `credential_manager::Backoff` derived its "jitter" deterministically
/// from the attempt counter (dead-code-marked and non-random), so a whole fleet
/// hitting the same rotation would retry in lockstep. This uses a per-call
/// `rand` draw so retries are decorrelated across nodes.
struct RegisterBackoff {
    attempt: u32,
}

impl RegisterBackoff {
    fn new() -> Self {
        Self { attempt: 0 }
    }

    fn reset(&mut self) {
        self.attempt = 0;
    }

    /// 5, 10, 20, 40, 60, 60 ... seconds, each with ±25% real jitter.
    fn next_delay(&mut self) -> Duration {
        let base_secs: u64 = match self.attempt {
            0 => 5,
            1 => 10,
            2 => 20,
            3 => 40,
            _ => 60,
        };
        self.attempt = self.attempt.saturating_add(1);

        let base_ms = base_secs * 1000;
        let jitter_span = (base_ms as f64 * 0.25) as i64;
        let jitter = rand::thread_rng().gen_range(-jitter_span..=jitter_span);
        let ms = (base_ms as i64 + jitter).max(1000) as u64;
        Duration::from_millis(ms)
    }
}

/// Slow fixed cadence for re-probing a terminal `Denied` realm.
///
/// A 403 is terminal for automatic recovery, but the realm may be re-provisioned
/// out of band. Re-probe on a slow, fixed cadence (never a tight loop) so the
/// node self-heals if the operator fixes the realm, without hammering AIS.
const DENIED_REPROBE_INTERVAL: Duration = Duration::from_secs(300);

// ---------------------------------------------------------------------------
// Controller
// ---------------------------------------------------------------------------

/// Owns the credential and coordinates re-acquire. Spawned once by
/// `ActrNode::start()` when the membership controller is enabled.
pub struct MembershipController {
    engine: CredentialManager,
    credential_tx: watch::Sender<Arc<PublishedCredential>>,
    report_rx: mpsc::Receiver<MembershipReport>,
    /// Woken after a new credential is published so the socket driver resets its
    /// backoff generation and reconnects with the fresh credential. Ordering is
    /// strict: publish happens-before this wake.
    wake_socket: Arc<dyn Fn() + Send + Sync>,
    shutdown: CancellationToken,
}

impl MembershipController {
    /// Build the controller and its [`MembershipHandle`].
    ///
    /// `seed` is the gen-0 credential (the pre-injected / initial-registration
    /// credential). It is published immediately so the socket driver has a
    /// credential to hand out before any report arrives.
    ///
    /// `wake_socket` is invoked (after each publish) to reset the socket
    /// driver's backoff generation and wake it — this is the strictly-ordered
    /// "publish happens-before wake" edge.
    pub fn new(
        engine: CredentialManager,
        seed: PublishedCredential,
        wake_socket: Arc<dyn Fn() + Send + Sync>,
        shutdown: CancellationToken,
    ) -> (Self, MembershipHandle) {
        let (credential_tx, credential_rx) = watch::channel(Arc::new(seed));
        let (report_tx, report_rx) = mpsc::channel(64);

        let handle = MembershipHandle {
            credential_rx,
            report_tx,
        };
        let controller = Self {
            engine,
            credential_tx,
            report_rx,
            wake_socket,
            shutdown,
        };
        (controller, handle)
    }

    /// The current published credential generation.
    fn current_generation(&self) -> u64 {
        self.credential_tx.borrow().generation
    }

    /// Publish a freshly minted credential, THEN wake the socket driver.
    ///
    /// Strict ordering: the `watch` send completes (so any consumer that wakes
    /// observes the new generation) before `wake_socket` fires. A consumer that
    /// parked on `wait_for(gen > stale)` therefore never rebuilds the URL from
    /// the old credential.
    fn publish_then_wake(&self, published: PublishedCredential) {
        let generation = published.generation;
        // 1. publish (data channel)
        self.credential_tx.send_replace(Arc::new(published));
        // 2. wake (control edge) — happens-after publish by construction
        (self.wake_socket)();
        tracing::info!(generation, "membership: published new credential and woke socket");
    }

    /// Run the coordinator loop until shutdown.
    ///
    /// This is the ONLY driver of re-acquire. It:
    /// - spawns a proactive expiry timer that reports `Rejected` at ~80% of the
    ///   credential lifetime (per-node jittered),
    /// - consumes reports, applying single-flight generation fencing,
    /// - on a fresh re-acquire, publishes then wakes the socket,
    /// - on a terminal `RealmDenied`, enters a slow re-probe cadence.
    pub async fn run(mut self) {
        tracing::info!("membership controller loop started");

        let mut backoff = RegisterBackoff::new();
        // The expiry timer runs as a sibling task that feeds the SAME report
        // channel, so the coordinator has a single consumption point.
        let expiry_report_tx = {
            // Re-derive a sender by cloning the watch's paired mpsc is not
            // possible here; instead the expiry timer is spawned by the caller
            // via `spawn_expiry_timer`. See `MembershipController::spawn`.
            None::<mpsc::Sender<MembershipReport>>
        };
        let _ = expiry_report_tx;

        loop {
            let report = tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => {
                    tracing::info!("membership controller loop shutting down");
                    return;
                }
                maybe = self.report_rx.recv() => match maybe {
                    Some(report) => report,
                    None => {
                        tracing::info!("membership report channel closed; controller exiting");
                        return;
                    }
                }
            };

            let current = self.current_generation();

            // Drop reports that are already handled: their stale generation is
            // behind the credential we have already published.
            if report.stale_generation < current {
                tracing::debug!(
                    stale = report.stale_generation,
                    current,
                    "membership: dropping already-handled report"
                );
                continue;
            }

            match report.verdict {
                AuthVerdict::RealmDenied => {
                    self.enter_denied(&mut backoff).await;
                    // enter_denied only returns once it has recovered (published
                    // a fresh credential) or shutdown fired.
                }
                AuthVerdict::Rejected => {
                    self.handle_rejection(report.stale_generation, &mut backoff)
                        .await;
                }
            }
        }
    }

    /// Single-flight, generation-fenced re-acquire for a `Rejected` verdict.
    ///
    /// Called with the stale generation being replaced. Because the coordinator
    /// loop consumes reports one at a time and re-acquire runs to completion
    /// before the next report is read, concurrent same-generation reports queued
    /// behind this one are naturally coalesced: by the time they are read their
    /// `stale_generation` is behind `current` and they are dropped. A DISTINCT
    /// higher generation that later fails advances the backoff ladder.
    async fn handle_rejection(&mut self, stale_generation: u64, backoff: &mut RegisterBackoff) {
        if self.engine.phase().await == SessionPhase::RealmUnavailable {
            tracing::warn!("membership: rejection while realm unavailable; entering denied");
            self.enter_denied(backoff).await;
            return;
        }

        tracing::warn!(
            stale_generation,
            "membership: credential rejected; re-acquiring (single-flight)"
        );

        match self.engine.reacquire().await {
            ReacquireOutcome::Renewed(published) => {
                backoff.reset();
                self.publish_then_wake(published);
            }
            ReacquireOutcome::Denied => {
                tracing::error!(
                    stale_generation,
                    "membership: realm denied during re-acquire; entering denied phase"
                );
                self.enter_denied(backoff).await;
            }
            ReacquireOutcome::Deferred(reason) => {
                // Transient failure: advance the jittered backoff ladder, then
                // let the loop pick up the next report (or re-report on timer).
                let delay = backoff.next_delay();
                tracing::warn!(
                    reason,
                    delay_ms = delay.as_millis() as u64,
                    "membership: re-acquire deferred; backing off before next attempt"
                );
                tokio::select! {
                    _ = self.shutdown.cancelled() => {}
                    _ = tokio::time::sleep(delay) => {
                        // Re-drive by synthesizing another attempt at the same
                        // stale generation, still single-flight via the loop.
                        self.retry_after_backoff(stale_generation, backoff).await;
                    }
                }
            }
        }
    }

    /// Retry re-acquire after a backoff sleep, staying generation-fenced.
    async fn retry_after_backoff(&mut self, stale_generation: u64, backoff: &mut RegisterBackoff) {
        if self.current_generation() > stale_generation {
            // Someone else already advanced us — nothing to do.
            return;
        }
        match self.engine.reacquire().await {
            ReacquireOutcome::Renewed(published) => {
                backoff.reset();
                self.publish_then_wake(published);
            }
            ReacquireOutcome::Denied => self.enter_denied(backoff).await,
            ReacquireOutcome::Deferred(reason) => {
                let delay = backoff.next_delay();
                tracing::warn!(reason, "membership: re-acquire still deferred; will retry on next signal");
                tokio::select! {
                    _ = self.shutdown.cancelled() => {}
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        }
    }

    /// Terminal `Denied` phase: stop, emit a loud metric, and re-probe the realm
    /// on a slow fixed cadence (never a tight loop) so the node self-heals if the
    /// realm is re-provisioned out of band.
    async fn enter_denied(&mut self, backoff: &mut RegisterBackoff) {
        self.engine.mark_realm_unavailable().await;
        tracing::error!(
            error_category = "membership_denied",
            severity = 10,
            reprobe_secs = DENIED_REPROBE_INTERVAL.as_secs(),
            "membership: realm denied — entering terminal Denied phase (slow re-probe)"
        );

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => return,
                _ = tokio::time::sleep(DENIED_REPROBE_INTERVAL) => {}
            }

            tracing::info!("membership: re-probing realm after denied cooldown");
            match self.engine.reacquire().await {
                ReacquireOutcome::Renewed(published) => {
                    tracing::info!("membership: realm recovered; leaving denied phase");
                    self.engine.mark_active().await;
                    backoff.reset();
                    self.publish_then_wake(published);
                    return;
                }
                ReacquireOutcome::Denied => {
                    tracing::warn!("membership: realm still denied; staying in denied phase");
                }
                ReacquireOutcome::Deferred(reason) => {
                    tracing::warn!(reason, "membership: realm re-probe deferred; staying in denied phase");
                }
            }
        }
    }
}

/// Spawn the controller loop and its proactive expiry timer.
///
/// Returns immediately; both tasks run until `shutdown` fires. The expiry timer
/// sleeps to `expires_at * 0.8 ± jitter`, sends a `Rejected` report so the
/// coordinator proactively renews, and re-arms after every publish (it re-reads
/// the watch each cycle).
pub fn spawn_membership_controller(
    controller: MembershipController,
    handle: &MembershipHandle,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut credential_rx = handle.credential_rx();
    let report_tx = handle.report_tx();
    let shutdown = controller.shutdown.clone();

    let expiry_task = tokio::spawn(async move {
        run_expiry_timer(&mut credential_rx, report_tx, shutdown).await;
    });

    let loop_task = tokio::spawn(controller.run());
    vec![loop_task, expiry_task]
}

/// Proactive expiry scheduler.
///
/// Parks on the `watch`; each time the credential changes it re-arms a timer to
/// ~80% of the remaining lifetime (with per-node jitter for fleet
/// decorrelation). When the timer fires it reports `Rejected` for the current
/// generation so the coordinator renews BEFORE the credential dies — the
/// scheduler the code always promised but never wired.
async fn run_expiry_timer(
    credential_rx: &mut watch::Receiver<Arc<PublishedCredential>>,
    report_tx: mpsc::Sender<MembershipReport>,
    shutdown: CancellationToken,
) {
    loop {
        let (generation, sleep_for) = {
            let published = credential_rx.borrow_and_update();
            (published.generation, proactive_renew_delay(&published))
        };

        let timer = async {
            match sleep_for {
                Some(d) => tokio::time::sleep(d).await,
                // No known expiry — park until the credential changes.
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            biased;
            _ = shutdown.cancelled() => return,
            changed = credential_rx.changed() => {
                if changed.is_err() {
                    return; // controller gone
                }
                // credential rotated — recompute the delay on the next loop.
                continue;
            }
            _ = timer => {
                tracing::info!(generation, "membership: proactive expiry timer fired; requesting renew");
                if report_tx
                    .send(MembershipReport {
                        verdict: AuthVerdict::Rejected,
                        stale_generation: generation,
                    })
                    .await
                    .is_err()
                {
                    return; // controller gone
                }
                // Wait for the credential to rotate before re-arming so we do
                // not spin reporting the same generation.
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => return,
                    changed = credential_rx.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }
}

/// Compute the proactive-renew delay: sleep to ~80% of the remaining lifetime,
/// with ±10% per-node jitter. Returns `None` when there is no usable expiry.
fn proactive_renew_delay(published: &PublishedCredential) -> Option<Duration> {
    let expires_at = published.credential_expires_at?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let remaining = expires_at.seconds - now;
    if remaining <= 0 {
        // Already expired — renew almost immediately (small jittered nudge to
        // avoid a fleet thundering herd).
        let jitter_ms = rand::thread_rng().gen_range(0..=2000);
        return Some(Duration::from_millis(jitter_ms));
    }
    let base = (remaining as f64 * 0.8).max(1.0);
    let jitter_span = base * 0.1;
    let jitter = rand::thread_rng().gen_range(-jitter_span..=jitter_span);
    let secs = (base + jitter).max(1.0) as u64;
    Some(Duration::from_secs(secs))
}

#[cfg(test)]
#[path = "membership_tests.rs"]
mod tests;
