//! Recovery-supervisor latency benchmarks (RFC-0400 / PR #399, new-branch side).
//!
//! This file carries two of the four quantitative-validation metrics for the
//! recovery state-machine refactor:
//!
//! * **M1 — acceptance latency.** Wall-clock time from a
//!   [`NetworkEventHandle`] call (`force_reconnect`) to its `oneshot` reply,
//!   against a mock [`NetworkEventProcessor`] whose effect performs a real
//!   (unpaused) `tokio::time::sleep(50ms)` to stand in for I/O.
//! * **M2 — enqueue-to-decision latency.** Wall-clock time from submitting a
//!   fact on the supervisor's request channel to the moment its
//!   `policy_revision` bump is observable on the `watch`-based status
//!   stream, against a no-op (immediately-successful) processor.
//!
//! The other two metrics do not fit this file:
//!
//! * **M3** (pure `translate()` throughput) calls `View`/`Input`/`translate`,
//!   which are `pub(crate)` — invisible to a `benches/` binary, which only
//!   links against the crate's public API. It lives as an in-tree, ignored
//!   unit test: `core/hyper/src/lifecycle/recovery_policy/translate_bench.rs`.
//!   Run it with:
//!   `cargo test --release -p actr-hyper --lib translate_bench:: -- --ignored --nocapture`
//! * **M4** (failure-backoff behavior under paused time) needs
//!   `#[tokio::test(start_paused = true)]`, which this `harness = false`
//!   binary's hand-rolled runtime cannot provide. It lives at
//!   `core/hyper/tests/recovery_backoff.rs`. Run it with:
//!   `cargo test --release -p actr-hyper --test recovery_backoff -- --ignored --nocapture`
//!
//! ## Honesty note on M1 (read before citing this number)
//!
//! M1 is **not** a same-path optimization measurement — it quantifies a
//! **semantic change** documented in the RFC-0400 compatibility section:
//! `NetworkEventHandle` acceptance now resolves "when the fact is accepted
//! and reconciled, not when the recovery effect completes". Concretely:
//!
//! * new branch: the reconciler answers the caller's `oneshot` as soon as one
//!   synchronous `translate()` round completes; the effect (including its
//!   simulated 50ms I/O) runs in a spawned task the caller never awaits.
//! * old branch (pre-#399, `origin/main`): the reconciler is a 400ms
//!   fixed-window *settle/debounce* batcher — it coalesces requests arriving
//!   within `NETWORK_EVENT_SETTLE_WINDOW`, selects one action, `.await`s the
//!   processor's action to completion, and only then replies to every queued
//!   caller. So the old-side number is dominated by that unconditional 400ms
//!   settle window, not by the 50ms effect alone; see the companion report
//!   for the measured old-side driver's numbers (built and run from a
//!   detached `origin/main` worktree, not part of this crate).
//!
//! M1's ~50ms simulated I/O sleep is the *benchmarked workload itself* (a
//! stand-in for a real reconnect's network round trip), not a
//! test-coordination sleep.
//!
//! ## Methodology
//!
//! Release build; a single-threaded (`current_thread`) tokio runtime this
//! binary owns (`harness = false`, matching `dispatch_throughput.rs`'s
//! precedent of a hand-rolled runtime — the reconciler and its spawned
//! effect/timer tasks are genuinely concurrent, so criterion's synchronous
//! harness does not apply). `current_thread` rather than a multi-worker
//! runtime is a deliberate choice, not the default: see the rationale on
//! `main`'s runtime builder below (a multi-worker runtime pinned to one core
//! via `taskset` was found to reproducibly stall). Real (unpaused) time
//! throughout: M1's 50ms sleep and M2's channel/watch round trips are actual
//! wall-clock waits, not `tokio::time::pause()`-driven.
//!
//! Each metric runs `RUNS` independent rounds (a fresh reconciler + handle per
//! round, so no state leaks across rounds), each collecting `SAMPLES`
//! measurements after a short discarded warmup. Per round we report
//! p50/p95/p99 (µs); across rounds we report the mean and standard deviation
//! of each percentile, so a reader can check that run-to-run noise (σ) is
//! small relative to the effect being measured (|Δ| between old and new, or
//! between M1 and M2).
//!
//! M1 samples are paced: after each `force_reconnect` call we wait (via a
//! dedicated completion channel from the mock processor — not a sleep) for
//! that round's simulated effect to finish before issuing the next one, so
//! every sample independently triggers a *fresh* 50ms effect instead of being
//! coalesced into an already-pending one. This pacing wait happens strictly
//! after the measured interval and is excluded from the sample.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use actr_hyper::lifecycle::{
    NetworkAvailability, NetworkEvent, NetworkEventHandle, NetworkEventProcessor,
    NetworkEventRequest, NetworkSnapshot, NetworkTransportFlags, ReconnectReason, SupervisorStatus,
    run_network_event_reconciler, run_network_event_reconciler_with_status,
};

// ── shared stats plumbing ───────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct Percentiles {
    p50_us: u64,
    p95_us: u64,
    p99_us: u64,
}

fn percentile_us(sorted: &[u64], p: f64) -> u64 {
    let n = sorted.len();
    assert!(n > 0, "no samples to summarize");
    let rank = (p * (n - 1) as f64).round() as usize;
    sorted[rank.min(n - 1)]
}

fn summarize(mut samples_us: Vec<u64>) -> Percentiles {
    samples_us.sort_unstable();
    Percentiles {
        p50_us: percentile_us(&samples_us, 0.50),
        p95_us: percentile_us(&samples_us, 0.95),
        p99_us: percentile_us(&samples_us, 0.99),
    }
}

fn mean_u64(xs: &[u64]) -> f64 {
    xs.iter().sum::<u64>() as f64 / xs.len() as f64
}

fn stddev_u64(xs: &[u64]) -> f64 {
    let m = mean_u64(xs);
    (xs.iter().map(|&x| (x as f64 - m).powi(2)).sum::<f64>() / xs.len() as f64).sqrt()
}

fn print_runs(label: &str, runs: &[Percentiles]) {
    println!("  run |   p50(us) |   p95(us) |   p99(us)");
    for (i, p) in runs.iter().enumerate() {
        println!(
            "  {:>3} | {:>9} | {:>9} | {:>9}",
            i + 1,
            p.p50_us,
            p.p95_us,
            p.p99_us
        );
    }
    let p50s: Vec<u64> = runs.iter().map(|p| p.p50_us).collect();
    let p95s: Vec<u64> = runs.iter().map(|p| p.p95_us).collect();
    let p99s: Vec<u64> = runs.iter().map(|p| p.p99_us).collect();
    println!(
        "{label}: p50 mean={:.1}us sigma={:.1}us | p95 mean={:.1}us sigma={:.1}us | p99 mean={:.1}us sigma={:.1}us",
        mean_u64(&p50s),
        stddev_u64(&p50s),
        mean_u64(&p95s),
        stddev_u64(&p95s),
        mean_u64(&p99s),
        stddev_u64(&p99s),
    );
}

// ── M1: acceptance latency against a real (unpaused) 50ms simulated effect ─

const M1_EFFECT_IO: Duration = Duration::from_millis(50);
const M1_RUNS: usize = 3;
const M1_WARMUP: usize = 3;
const M1_SAMPLES: usize = 200;

/// A processor whose `force_reconnect` performs one real 50ms `sleep` — the
/// *benchmarked I/O workload*, not a coordination sleep — then signals
/// completion on `done_tx` so the bench loop can pace fresh samples.
struct SleepingProcessor {
    io: Duration,
    done_tx: mpsc::UnboundedSender<()>,
}

#[async_trait]
impl NetworkEventProcessor for SleepingProcessor {
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
        tokio::time::sleep(self.io).await;
        let _ = self.done_tx.send(());
        Ok(())
    }
}

async fn m1_one_run() -> Percentiles {
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<()>();
    let processor: Arc<dyn NetworkEventProcessor> = Arc::new(SleepingProcessor {
        io: M1_EFFECT_IO,
        done_tx,
    });
    let (event_tx, event_rx) = mpsc::channel::<NetworkEventRequest>(8);
    let shutdown = CancellationToken::new();
    let handle = NetworkEventHandle::new(event_tx);
    let recon = tokio::spawn(run_network_event_reconciler(
        event_rx,
        processor,
        shutdown.clone(),
    ));

    let mut samples_us = Vec::with_capacity(M1_SAMPLES);
    for i in 0..(M1_WARMUP + M1_SAMPLES) {
        let t0 = Instant::now();
        handle
            .force_reconnect(ReconnectReason::ManualReconnect)
            .await
            .expect("acceptance reply");
        let dt = t0.elapsed();
        // Pacing wait (excluded from the sample): let this round's simulated
        // effect finish before the next round, so every sample starts a
        // genuinely fresh recovery cycle instead of being coalesced into one
        // already in flight.
        done_rx.recv().await.expect("effect completion signal");
        if i >= M1_WARMUP {
            samples_us.push(dt.as_micros() as u64);
        }
    }

    shutdown.cancel();
    let _ = recon.await;
    summarize(samples_us)
}

// ── M2: enqueue -> policy_revision jump, no-op (immediately successful) effect

const M2_RUNS: usize = 5;
const M2_WARMUP: usize = 10;
const M2_SAMPLES: usize = 600;
const M2_SOURCE_EPOCH: u64 = 1;

struct NoopProcessor;

#[async_trait]
impl NetworkEventProcessor for NoopProcessor {
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
}

/// A network-available snapshot whose `route_fingerprint` alternates between
/// two values on every call, so each snapshot is structurally "material"
/// relative to the previous one and `translate()` advances `policy_revision`
/// by exactly one every time (see `translate_snapshot`'s Online→Online
/// self-loop-with-material-route-change row).
fn m2_snapshot(seq: u64) -> NetworkSnapshot {
    let mut transport = NetworkTransportFlags::default();
    if seq.is_multiple_of(2) {
        transport.wifi = true;
    } else {
        transport.cellular = true;
    }
    NetworkSnapshot {
        sequence: seq,
        availability: NetworkAvailability::Available,
        transport,
        is_expensive: false,
        is_constrained: false,
    }
}

async fn m2_one_run() -> Percentiles {
    let (event_tx, event_rx) = mpsc::channel::<NetworkEventRequest>(64);
    let (status_tx, mut status_rx) = watch::channel(SupervisorStatus::default());
    let processor: Arc<dyn NetworkEventProcessor> = Arc::new(NoopProcessor);
    let shutdown = CancellationToken::new();
    let recon = tokio::spawn(run_network_event_reconciler_with_status(
        event_rx,
        processor,
        shutdown.clone(),
        status_tx,
    ));

    let mut samples_us = Vec::with_capacity(M2_SAMPLES);
    let mut base_rev = status_rx.borrow().policy_revision;
    for i in 0..(M2_WARMUP + M2_SAMPLES) {
        let seq = i as u64 + 1;
        let (result_tx, _result_rx) = oneshot::channel();
        let want_rev = base_rev + 1;

        let t0 = Instant::now();
        event_tx
            .send(NetworkEventRequest {
                event: NetworkEvent::NetworkPathChanged {
                    snapshot: m2_snapshot(seq),
                },
                result_tx,
                source_epoch: M2_SOURCE_EPOCH,
                observed_at: tokio::time::Instant::now(),
            })
            .await
            .expect("enqueue fact");
        loop {
            if status_rx.borrow().policy_revision >= want_rev {
                break;
            }
            status_rx.changed().await.expect("status stream alive");
        }
        let dt = t0.elapsed();

        base_rev = want_rev;
        if i >= M2_WARMUP {
            samples_us.push(dt.as_micros() as u64);
        }
    }

    shutdown.cancel();
    let _ = recon.await;
    summarize(samples_us)
}

// ── entry point ──────────────────────────────────────────────────────────

fn main() {
    // Single-threaded runtime, deliberately: a multi-worker runtime pinned to
    // one logical core via `taskset -c N` (the documented, reproducible way
    // to run this bench) was found to reproducibly stall -- the reactor/timer
    // driver thread appears to starve against its sibling worker under
    // single-core contention, so a spawned effect's `sleep(50ms)` completion
    // is never delivered. A `current_thread` runtime sidesteps that hazard
    // entirely (there is only one OS thread to pin) and, as a side benefit,
    // removes cross-thread wake latency from the very acceptance-latency
    // number M1 is measuring -- the reconciler, the spawned effect task, and
    // this loop all run cooperatively on the same thread.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    rt.block_on(async {
        println!("# recovery_latency (new branch)");
        println!(
            "M1 config: effect_io={:?} runs={M1_RUNS} warmup={M1_WARMUP} samples={M1_SAMPLES}",
            M1_EFFECT_IO
        );
        println!("M2 config: runs={M2_RUNS} warmup={M2_WARMUP} samples={M2_SAMPLES}");
        println!();

        println!(
            "## M1 — acceptance latency (oneshot resolves at acceptance, not effect completion)"
        );
        let mut m1_runs = Vec::with_capacity(M1_RUNS);
        for r in 0..M1_RUNS {
            let p = m1_one_run().await;
            println!(
                "  run {}: p50={}us p95={}us p99={}us",
                r + 1,
                p.p50_us,
                p.p95_us,
                p.p99_us
            );
            m1_runs.push(p);
        }
        print_runs("M1", &m1_runs);
        println!();

        println!(
            "## M2 — enqueue-to-decision latency (fact submit -> policy_revision jump on status watch)"
        );
        let mut m2_runs = Vec::with_capacity(M2_RUNS);
        for r in 0..M2_RUNS {
            let p = m2_one_run().await;
            println!(
                "  run {}: p50={}us p95={}us p99={}us",
                r + 1,
                p.p50_us,
                p.p95_us,
                p.p99_us
            );
            m2_runs.push(p);
        }
        print_runs("M2", &m2_runs);
    });
}
