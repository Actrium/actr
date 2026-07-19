//! M3 — pure `translate()` throughput micro-benchmark.
//!
//! `translate`, `View`, and `Input` are `pub(crate)`: they are the RFC-0400
//! policy core's internal vocabulary, deliberately not part of the crate's
//! public API. A `benches/*.rs` binary is a separate compilation unit that
//! only links against `actr_hyper`'s public surface, so it cannot name any of
//! these types — `core/hyper/benches/recovery_latency.rs` (M1/M2) documents
//! this and points here.
//!
//! This lives as an in-tree, `#[ignore]`d unit test instead: a child module
//! of `translate`, so it has the same `super::*` access as
//! `translate_tests.rs`. It is deliberately *not* a normal `#[test]` that
//! runs in `cargo test` — it is a throughput measurement, not a correctness
//! check — hence `#[ignore]`.
//!
//! ## Run it
//!
//! ```text
//! cargo test --release -p actr-hyper --lib translate_bench:: -- --ignored --nocapture
//! ```
//!
//! ## Method
//!
//! One fixed `View` (a representative mid-flight snapshot: active/foreground/
//! online, a Reconnect effect in flight at revision 5) and one representative
//! mix of seven `Input`s spanning the categories the task calls out —
//! `NetworkSnapshot` online/offline, `AppEnteredForeground`/`Background`,
//! `EffectCompleted` success/failure, and `RecoveryRequested` — are built
//! once. The `View` is never mutated by applying the returned `Decision`
//! (translation is queried, not driven), so every call measures the same
//! `translate()` cost regardless of accumulated policy state; this isolates
//! the reducer's own per-call cost from the surrounding supervisor's
//! bookkeeping (which M1/M2 already cover end-to-end).
//!
//! Each round calls `translate()` `ITERS_PER_ROUND` times cycling through the
//! seven inputs, accumulating a cheap checksum of each `Decision`'s shape
//! into `std::hint::black_box` so the optimizer cannot prove the result is
//! unused and elide the calls. `ROUNDS` rounds are timed independently; the
//! median round's rate is the reported throughput.

use std::hint::black_box;
use std::time::Instant;

use super::super::classification::FixedEntropy;
use super::*;

const ITERS_PER_ROUND: usize = 300_000;
const ROUNDS: usize = 7;

fn representative_view() -> View {
    let mut view = View::initial();
    view.recovery_mode = RecoveryModeState::Active;
    view.app_phase = AppPhaseState::Foreground;
    view.network_path = NetworkPathState::Online;
    view.recovery_intent = RecoveryIntentState::ReconnectPending;
    view.execution = ExecutionState::Reconnecting;
    view.policy_revision = 5;
    view.recovery_record = Some(PendingRecord::recovery(5, RecoveryStrength::Reconnect));
    view.effect = Some(EffectContext {
        action_id: 1,
        kind: EffectKind::Reconnect,
        captured_revision: 5,
        cancel_reason: None,
    });
    view.last_snapshot = Some(AcceptedSnapshot {
        source_epoch: 1,
        sequence: 1,
        semantic_path: SemanticPath::Unknown,
        route_fingerprint: 0,
    });
    view.live_signaling_generation = Some(3);
    view
}

/// Seven inputs: `NetworkSnapshot` online/offline, `AppEntered{Fore,Back}ground`,
/// `EffectCompleted` success/failure, `RecoveryRequested` — the categories the
/// spec calls out.
fn representative_inputs() -> Vec<Input> {
    vec![
        Input::NetworkSnapshot {
            source_epoch: 1,
            sequence: 2,
            semantic_path: SemanticPath::Online,
            route_fingerprint: 1,
        },
        Input::NetworkSnapshot {
            source_epoch: 1,
            sequence: 3,
            semantic_path: SemanticPath::Offline,
            route_fingerprint: 1,
        },
        Input::AppEnteredForeground,
        Input::AppEnteredBackground,
        Input::RecoveryRequested {
            minimum: RecoveryStrength::Reconnect,
            reason: RecoveryRequestReason::ManualReconnect,
        },
        Input::EffectCompleted {
            action_id: 1,
            kind: EffectKind::Reconnect,
            policy_revision: 5,
            outcome: EffectOutcome::Succeeded,
        },
        Input::EffectCompleted {
            action_id: 1,
            kind: EffectKind::Reconnect,
            policy_revision: 5,
            outcome: EffectOutcome::Failed {
                diagnosis: EffectDiagnosis::PathUnreachable {
                    stage: "probe".to_string(),
                },
            },
        },
    ]
}

#[test]
#[ignore = "throughput microbenchmark, not a correctness check; \
            run with `--release ... -- --ignored --nocapture`"]
fn translate_throughput_representative_mix() {
    let view = representative_view();
    let inputs = representative_inputs();
    let config = PolicyConfig::defaults();
    let now = Duration::from_secs(1);

    let mut round_rates = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        let mut entropy = FixedEntropy::constant(0.37);
        let mut sink: u64 = 0;
        let start = Instant::now();
        for i in 0..ITERS_PER_ROUND {
            let input = &inputs[i % inputs.len()];
            let d = translate(&view, input, now, &config, &mut entropy);
            sink = sink
                .wrapping_add(d.machine_inputs.len() as u64)
                .wrapping_add(d.timers.len() as u64)
                .wrapping_add(d.gate_triggers.len() as u64)
                .wrapping_add(d.cancels.len() as u64);
        }
        let elapsed = start.elapsed();
        black_box(sink);
        let rate = ITERS_PER_ROUND as f64 / elapsed.as_secs_f64();
        let ns_per_decision = elapsed.as_nanos() as f64 / ITERS_PER_ROUND as f64;
        println!(
            "round {}: {:?} total, {:.0} decisions/sec, {:.1} ns/decision",
            round + 1,
            elapsed,
            rate,
            ns_per_decision
        );
        round_rates.push(rate);
    }

    round_rates.sort_by(|a, b| a.partial_cmp(b).expect("no NaN rates"));
    let median_rate = round_rates[ROUNDS / 2];
    println!(
        "median: {:.0} decisions/sec ({:.1} ns/decision)",
        median_rate,
        1e9 / median_rate
    );
}
