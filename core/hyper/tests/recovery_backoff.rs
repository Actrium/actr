//! M4 — failure-backoff behavior under paused (virtual) time.
//!
//! Deterministically drives an always-failing `Reconnect` effect
//! (`EffectDiagnosis::PathUnreachable`, availability family) across ten
//! virtual minutes and records: how many times the effect actually
//! re-executes, the wall-clock (virtual) interval between successive
//! attempts, and whether the retry gate ever stops (parks).
//!
//! ## Run it
//!
//! This is `#[ignore]`d — it is a deterministic behavior probe, not a
//! correctness gate meant for every `cargo test` run (it prints its interval
//! series for a human to inspect, not just pass/fail):
//!
//! ```text
//! cargo test --release -p actr-hyper --test recovery_backoff -- --ignored --nocapture
//! ```
//!
//! ## Method: small-step `tokio::time::advance`
//!
//! `#[tokio::test(start_paused = true)]` freezes the runtime clock.
//! `tokio::time::advance(d)` is documented to fire every timer already
//! *armed* at or before the new instant, but a single large jump is unsafe
//! for **chained** timers: this supervisor's failure path re-arms a *new*
//! backoff deadline only after the previous one fires and its `Failed`
//! completion is processed (timer fires → reconciler drives the effect retry
//! → it fails → `translate()` arms the *next* backoff). A jump larger than
//! one backoff interval can race past a not-yet-armed follow-up timer and
//! silently under-count retries. We instead advance in a fixed step
//! (100ms — well under the shortest possible backoff interval, 500ms base
//! with up to -20% jitter ≈ 400ms) and yield after each step so the
//! reconciler, the effect task, and any newly-armed timer all get scheduled
//! before the next advance.
//!
//! ## Why "never parks" is asserted the way it is
//!
//! `RetryGateState`/`Verdict`/`classify()` are `pub(crate)` — invisible here,
//! since integration tests are a separate compilation unit linking only the
//! public API. The "never parks" claim instead rests on two legs documented
//! in the report this test supports:
//!
//! 1. **Source-level invariant** (cited, not re-derived here):
//!    `classify_reconnect` in `recovery_policy/classification.rs` parks only
//!    on `DiagnosisFamily::Precondition`; `PathUnreachable` is
//!    `DiagnosisFamily::Availability`, which that function maps
//!    unconditionally to `Verdict::Retry` — parking a Reconnect on a
//!    `PathUnreachable` failure is not reachable code, regardless of attempt
//!    count.
//! 2. **Empirical corroboration** (this test): a parked gate stops retrying
//!    (it waits for an explicit clearing trigger, not a timer), so if parking
//!    had occurred, attempts would cease well before the ten-minute mark.
//!    This test asserts the last recorded attempt lands within one capped
//!    backoff interval of the window's end, i.e. retries never stalled.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use actr_hyper::lifecycle::{
    NetworkEventHandle, NetworkEventProcessor, NetworkEventRequest, ReconnectReason,
    SupervisorStatus, run_network_event_reconciler_with_status,
};

/// A processor whose `force_reconnect` always fails, recording the (virtual)
/// instant of every call. The plain `Err(String)` return is mapped by the
/// shell's `diagnose_effect_error` to `EffectDiagnosis::PathUnreachable` —
/// the availability-family diagnosis this test targets — entirely through
/// the public `NetworkEventProcessor` contract, no internal access needed.
struct AlwaysUnreachableProcessor {
    call_times: Arc<Mutex<Vec<Instant>>>,
}

#[async_trait]
impl NetworkEventProcessor for AlwaysUnreachableProcessor {
    async fn process_network_available(&self) -> Result<(), String> {
        Ok(())
    }
    async fn process_network_lost(&self) -> Result<(), String> {
        Ok(())
    }
    async fn process_network_type_changed(
        &self,
        _is_wifi: bool,
        _is_cellular: bool,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn cleanup_connections(&self) -> Result<(), String> {
        Ok(())
    }

    async fn force_reconnect(&self) -> Result<(), String> {
        self.call_times.lock().expect("lock").push(Instant::now());
        Err("simulated path unreachable".to_string())
    }
}

const STEP: Duration = Duration::from_millis(100);
const TOTAL: Duration = Duration::from_secs(600);
/// Reconnect's documented backoff ceiling (`PolicyConfig::defaults().backoff.reconnect`):
/// base 500ms doubling to a cap of 30s. Used only to sanity-bound the
/// "still retrying near the end of the window" assertion below.
const MAX_BACKOFF_CEILING: Duration = Duration::from_secs(30);

#[tokio::test(start_paused = true)]
#[ignore = "10 virtual minutes of small-step time advance; a behavior probe, \
            not a per-commit correctness gate. Run explicitly with --ignored --nocapture."]
async fn availability_failure_retries_with_backoff_and_never_parks() {
    let call_times = Arc::new(Mutex::new(Vec::new()));
    let processor: Arc<dyn NetworkEventProcessor> = Arc::new(AlwaysUnreachableProcessor {
        call_times: call_times.clone(),
    });
    let (event_tx, event_rx) = mpsc::channel::<NetworkEventRequest>(8);
    let (status_tx, _status_rx) = tokio::sync::watch::channel(SupervisorStatus::default());
    let shutdown = CancellationToken::new();
    let handle = NetworkEventHandle::new(event_tx);

    let recon = tokio::spawn(run_network_event_reconciler_with_status(
        event_rx,
        processor,
        shutdown.clone(),
        status_tx,
    ));

    let window_start = Instant::now();
    handle
        .force_reconnect(ReconnectReason::ManualReconnect)
        .await
        .expect("acceptance reply");

    let mut advanced = Duration::ZERO;
    while advanced < TOTAL {
        tokio::time::advance(STEP).await;
        // Let the reconciler, the failing effect task, and any newly-armed
        // backoff timer run before the next step.
        tokio::task::yield_now().await;
        advanced += STEP;
    }

    shutdown.cancel();
    let _ = recon.await;

    let times = call_times.lock().expect("lock").clone();
    println!(
        "retries over {:?} virtual time: {} attempts",
        TOTAL,
        times.len()
    );
    let mut prev = window_start;
    let mut gaps = Vec::with_capacity(times.len());
    for (i, &t) in times.iter().enumerate() {
        let gap = t.saturating_duration_since(prev);
        println!(
            "  attempt {:>3}: t={:>9?}  gap={:>9?}",
            i + 1,
            t - window_start,
            gap
        );
        gaps.push(gap);
        prev = t;
    }

    assert!(
        times.len() >= 2,
        "expected multiple retries over a 10-minute window, got {}",
        times.len()
    );

    // Empirical "never parked" corroboration: a parked gate stops retrying
    // (it waits for an explicit clearing trigger, not a timer), so retries
    // continuing right up to the end of the window rules out a park having
    // occurred at any point in it. One backoff ceiling of slack absorbs the
    // last in-flight interval.
    let last = *times.last().expect("at least one attempt");
    let time_since_last = (window_start + TOTAL).saturating_duration_since(last);
    assert!(
        time_since_last <= MAX_BACKOFF_CEILING,
        "retries stalled {:?} before the window's end (>{:?} ceiling) -- \
         looks like the gate parked instead of continuing to back off",
        time_since_last,
        MAX_BACKOFF_CEILING,
    );

    // Sanity-check the shape against the documented curve (base 500ms
    // doubling per failure, exponent capped at 6, ceiling 30s, ±20% jitter):
    // `gaps[0]` is ~0 (window start -> first attempt, fired synchronously at
    // acceptance). `gaps[k]` for k=1..=6 is the pre-cap ramp, whose unjittered
    // delay is `500ms * 2^(k-1)`; because doubling (2x) always dominates the
    // ±20% jitter spread (worst case 2*0.8=1.6 > 1*1.0=1.0), this region must
    // still be strictly increasing even under adversarial jitter draws.
    // `gaps[k]` for k>=7 all share the same unjittered 30s ceiling
    // (exponent saturates at 6), so jitter alone decides each value: they
    // land independently in the jittered-cap band and are **not** expected
    // to be monotonic among themselves -- that oscillation is the intended
    // "sustained capped, jittered backoff" behavior, not a regression.
    let ramp_end = 6.min(gaps.len().saturating_sub(1));
    for k in 2..=ramp_end {
        assert!(
            gaps[k] + Duration::from_millis(50) >= gaps[k - 1],
            "ramp backoff interval shrank before reaching the cap: gap[{}]={:?} -> gap[{}]={:?}",
            k - 1,
            gaps[k - 1],
            k,
            gaps[k]
        );
    }
    let jittered_band_low = MAX_BACKOFF_CEILING
        .mul_f64(0.8)
        .saturating_sub(Duration::from_millis(200));
    let jittered_band_high = MAX_BACKOFF_CEILING + Duration::from_millis(200);
    for (k, gap) in gaps.iter().enumerate().skip(ramp_end + 1) {
        assert!(
            *gap >= jittered_band_low && *gap <= jittered_band_high,
            "capped backoff interval gap[{k}]={:?} falls outside the jittered \
             [{:?}, {:?}] band around the {:?} ceiling",
            gap,
            jittered_band_low,
            jittered_band_high,
            MAX_BACKOFF_CEILING
        );
    }
}
