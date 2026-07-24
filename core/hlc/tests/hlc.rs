//! RFC-0419 §6 test matrix for the optional HLC library.
//!
//! Every test drives a programmable physical clock; no real sleeping and no
//! `SystemTime` reads are involved, so the whole suite is deterministic.

use std::cell::Cell;
use std::rc::Rc;

use actr_hlc::{Clock, Error, RemoteError, State, Timestamp, UtcClock, validate_remote};

/// Programmable physical clock shared between the test body and the clock
/// under test.
#[derive(Clone)]
struct ScriptClock {
    now: Rc<Cell<i64>>,
}

impl ScriptClock {
    fn new(start_ms: i64) -> Self {
        Self {
            now: Rc::new(Cell::new(start_ms)),
        }
    }

    fn set(&self, now_ms: i64) {
        self.now.set(now_ms);
    }

    fn shift(&self, delta_ms: i64) {
        self.now.set(self.now.get() + delta_ms);
    }
}

impl UtcClock for ScriptClock {
    fn now_ms(&self) -> i64 {
        self.now.get()
    }
}

fn ts(physical_ms: i64, logical: u32) -> Timestamp {
    Timestamp {
        physical_ms,
        logical,
    }
}

/// Deterministic pseudo-random generator (64-bit LCG, upper bits).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

// --- §6: strict increase of consecutive local events under all UTC drivers -

#[test]
fn local_events_strictly_increase_while_utc_advances() {
    let utc = ScriptClock::new(1_000);
    let mut clock = Clock::new(utc.clone());
    let mut prev = clock.local_event().unwrap();
    for _ in 0..100 {
        utc.shift(7);
        let next = clock.local_event().unwrap();
        assert!(next > prev, "{next:?} <= {prev:?}");
        prev = next;
    }
}

#[test]
fn local_events_strictly_increase_while_utc_is_frozen() {
    let utc = ScriptClock::new(1_000);
    let mut clock = Clock::new(utc.clone());
    // (0, 0) initial state converges to the frozen reading first.
    assert_eq!(clock.local_event().unwrap(), ts(1_000, 0));
    // Then the logical counter absorbs every further event.
    let mut prev = ts(1_000, 0);
    for expected_logical in 1..=100 {
        let next = clock.local_event().unwrap();
        assert_eq!(next, ts(1_000, expected_logical));
        assert!(next > prev);
        prev = next;
    }
}

#[test]
fn local_events_strictly_increase_while_utc_runs_backwards() {
    let utc = ScriptClock::new(10_000);
    let mut clock = Clock::new(utc.clone());
    let mut prev = clock.local_event().unwrap();
    for _ in 0..100 {
        utc.shift(-3);
        let next = clock.local_event().unwrap();
        assert!(next > prev, "{next:?} <= {prev:?}");
        // The issued physical component never follows UTC backwards.
        assert!(next.physical_ms >= 10_000);
        prev = next;
    }
}

// --- §6: observe output strictly exceeds the remote event -----------------

#[test]
fn observe_output_exceeds_remote_and_previous_local_output() {
    let utc = ScriptClock::new(1_000);
    let mut clock = Clock::new(utc.clone());
    let local = clock.local_event().unwrap();

    // Remote ahead of local UTC.
    let ahead = ts(5_000, 41);
    let merged = clock.observe(ahead).unwrap();
    assert!(merged > ahead);
    assert!(merged > local);

    // Remote behind everything: output still strictly above both.
    let behind = ts(10, 3);
    let merged_again = clock.observe(behind).unwrap();
    assert!(merged_again > behind);
    assert!(merged_again > merged);
}

// --- deterministic interleaved sequence -----------------------------------

#[test]
fn interleaved_local_and_observe_sequence_is_strictly_monotonic() {
    let utc = ScriptClock::new(0);
    let mut clock = Clock::new(utc.clone());
    let mut rng = Lcg(0x5eed_1234_abcd_0042);
    let mut prev = clock.local_event().unwrap();
    for step in 0..10_000 {
        // Advance, rewind or freeze UTC between events.
        match rng.next() % 4 {
            0 => utc.shift((rng.next() % 50) as i64),
            1 => utc.shift(-((rng.next() % 20) as i64)),
            _ => {}
        }
        let next = if rng.next().is_multiple_of(2) {
            clock.local_event().unwrap()
        } else {
            let remote = ts(
                (utc.now_ms() + (rng.next() % 100) as i64).max(0),
                (rng.next() % 1_000) as u32,
            );
            clock.observe(remote).unwrap()
        };
        assert!(next > prev, "step {step}: {next:?} <= {prev:?}");
        prev = next;
    }
}

// --- §6: state export / restore round trip --------------------------------

#[test]
fn restoring_the_latest_exported_state_preserves_strict_monotonicity() {
    let utc = ScriptClock::new(2_000);
    let mut clock = Clock::new(utc.clone());
    for _ in 0..5 {
        clock.local_event().unwrap();
    }
    clock.observe(ts(2_500, 9)).unwrap();
    let state = clock.export_state();

    // Restart with UTC far behind the persisted state: the next output must
    // still be strictly greater than the state (RFC acceptance invariant).
    utc.set(100);
    let mut restored = Clock::from_state(utc.clone(), state);
    assert!(restored.local_event().unwrap() > state.last);

    // Same restart with observe as the first operation after recovery.
    let mut restored = Clock::from_state(utc.clone(), state);
    assert!(restored.observe(ts(50, 7)).unwrap() > state.last);
}

// --- §6: crash after export — unsafe restore, then the safe strategies ----

#[test]
fn stale_state_restore_reissues_already_used_timestamps() {
    let utc = ScriptClock::new(1_000); // frozen: worst case for duplicates
    let mut clock = Clock::new(utc.clone());
    clock.local_event().unwrap();
    let stale = clock.export_state();

    // Timestamps issued after the export and before the "crash".
    let after_export = clock.local_event().unwrap();
    let max_issued = clock.local_event().unwrap();

    // Restoring the stale state reproduces the risk the RFC warns about:
    // the clock re-issues a timestamp that was already handed out.
    let mut restored = Clock::from_state(utc.clone(), stale);
    let reissued = restored.local_event().unwrap();
    assert_eq!(reissued, after_export);
    assert!(reissued <= max_issued);
}

#[test]
fn persist_before_issue_prevents_reissue_after_crash() {
    let utc = ScriptClock::new(1_000); // frozen
    let mut clock = Clock::new(utc.clone());
    let mut latest_persisted = clock.export_state();
    let mut max_issued = latest_persisted.last;
    for _ in 0..10 {
        let issued = clock.local_event().unwrap();
        // Persist-before-issue: the advanced state is durable before the
        // timestamp is handed out, so no issued timestamp can outrun it.
        latest_persisted = clock.export_state();
        max_issued = issued;
    }

    let mut restored = Clock::from_state(utc.clone(), latest_persisted);
    assert!(restored.local_event().unwrap() > max_issued);
}

#[test]
fn physical_forward_jump_on_recovery_prevents_reissue() {
    const MAX_CLOCK_ERROR_MS: i64 = 5_000;
    let utc = ScriptClock::new(1_000); // frozen
    let mut clock = Clock::new(utc.clone());
    clock.local_event().unwrap();
    let stale = clock.export_state();
    let mut max_issued = stale.last;
    for _ in 0..10 {
        max_issued = clock.local_event().unwrap();
    }

    // All post-export issuance stayed within the clock-error window, so
    // jumping the restored physical component forward by at least that
    // window clears everything the stale state failed to cover.
    let jumped = State::new(ts(stale.last.physical_ms + MAX_CLOCK_ERROR_MS, 0));
    let mut restored = Clock::from_state(utc.clone(), jumped);
    assert!(restored.local_event().unwrap() > max_issued);
}

// --- §6: logical overflow carries into the physical component -------------

#[test]
fn logical_overflow_carries_into_the_physical_component() {
    let utc = ScriptClock::new(500); // frozen, behind the saturated state
    let saturated = ts(1_000, u32::MAX);
    let mut clock = Clock::from_state(utc.clone(), State::new(saturated));
    let carried = clock.local_event().unwrap();
    assert_eq!(carried, ts(1_001, 0));
    assert!(carried > saturated);

    // Same carry when observe must increment past u32::MAX.
    let mut clock = Clock::from_state(utc.clone(), State::new(ts(1_000, 3)));
    let merged = clock.observe(ts(1_000, u32::MAX)).unwrap();
    assert_eq!(merged, ts(1_001, 0));
}

// --- §6: physical overflow errors out and leaves the state unchanged ------

#[test]
fn physical_overflow_returns_error_and_leaves_state_unchanged() {
    let utc = ScriptClock::new(0); // frozen
    let saturated = State::new(ts(i64::MAX, u32::MAX));
    let mut clock = Clock::from_state(utc.clone(), saturated);
    let before = clock.export_state();

    assert_eq!(clock.local_event(), Err(Error::PhysicalOverflow));
    assert_eq!(clock.export_state(), before);

    assert_eq!(
        clock.observe(ts(i64::MAX, u32::MAX)),
        Err(Error::PhysicalOverflow)
    );
    assert_eq!(clock.export_state(), before);

    // Overflow through the remote branch: a fresh clock observing an
    // already-saturated remote timestamp.
    let mut fresh = Clock::new(utc.clone());
    let before = fresh.export_state();
    assert_eq!(
        fresh.observe(ts(i64::MAX, u32::MAX)),
        Err(Error::PhysicalOverflow)
    );
    assert_eq!(fresh.export_state(), before);
}

// --- §6: validation helper -------------------------------------------------

#[test]
fn validate_remote_rejects_negative_physical() {
    assert_eq!(
        validate_remote(1_000, ts(-1, 0), 500),
        Err(RemoteError::NegativePhysical)
    );
    assert_eq!(
        validate_remote(1_000, ts(i64::MIN, 7), 500),
        Err(RemoteError::NegativePhysical)
    );
}

#[test]
fn validate_remote_rejects_future_skew_beyond_threshold() {
    assert_eq!(
        validate_remote(1_000, ts(1_501, 0), 500),
        Err(RemoteError::FutureSkew { ahead_ms: 501 })
    );
    // Saturating subtraction keeps extreme inputs from overflowing.
    assert_eq!(
        validate_remote(i64::MIN, ts(i64::MAX, 0), 500),
        Err(RemoteError::FutureSkew { ahead_ms: i64::MAX })
    );
}

#[test]
fn validate_remote_accepts_timestamps_within_threshold() {
    // Exactly at the limit.
    assert_eq!(validate_remote(1_000, ts(1_500, 42), 500), Ok(()));
    // In the past.
    assert_eq!(validate_remote(1_000, ts(900, 0), 500), Ok(()));
    // The logical counter takes no part in skew policy.
    assert_eq!(validate_remote(1_000, ts(1_000, u32::MAX), 500), Ok(()));
}

// --- §6: fixed cross-platform test vectors --------------------------------

#[test]
fn fixed_local_event_vectors() {
    // (name, state, now, expected output == expected new state)
    let vectors: [(&str, Timestamp, i64, Timestamp); 4] = [
        (
            "utc behind state: logical increments",
            ts(1_000, 5),
            900,
            ts(1_000, 6),
        ),
        (
            "utc equals state: logical increments",
            ts(1_000, 5),
            1_000,
            ts(1_000, 6),
        ),
        (
            "utc ahead of state: logical resets",
            ts(1_000, 5),
            2_000,
            ts(2_000, 0),
        ),
        (
            "fresh state converges to utc",
            ts(0, 0),
            1_234,
            ts(1_234, 0),
        ),
    ];
    for (name, state, now, expected) in vectors {
        let mut clock = Clock::from_state(ScriptClock::new(now), State::new(state));
        assert_eq!(clock.local_event().unwrap(), expected, "{name}");
        assert_eq!(clock.export_state().last, expected, "{name}: state");
    }
}

#[test]
fn fixed_observe_vectors() {
    // (name, state, now, remote, expected output == expected new state)
    let vectors: [(&str, Timestamp, i64, Timestamp, Timestamp); 6] = [
        (
            "physical tie, remote logical larger",
            ts(1_000, 5),
            900,
            ts(1_000, 9),
            ts(1_000, 10),
        ),
        (
            "physical tie, local logical larger",
            ts(1_000, 7),
            900,
            ts(1_000, 2),
            ts(1_000, 8),
        ),
        (
            "local physical wins",
            ts(1_000, 5),
            900,
            ts(800, 7),
            ts(1_000, 6),
        ),
        (
            "remote physical wins",
            ts(1_000, 5),
            900,
            ts(1_500, 7),
            ts(1_500, 8),
        ),
        (
            "utc ties remote above local state",
            ts(500, 5),
            1_500,
            ts(1_500, 7),
            ts(1_500, 8),
        ),
        (
            "utc wins over both",
            ts(1_000, 5),
            3_000,
            ts(2_000, 7),
            ts(3_000, 0),
        ),
    ];
    for (name, state, now, remote, expected) in vectors {
        let mut clock = Clock::from_state(ScriptClock::new(now), State::new(state));
        assert_eq!(clock.observe(remote).unwrap(), expected, "{name}");
        assert_eq!(clock.export_state().last, expected, "{name}: state");
    }
}

// --- ordering and construction ---------------------------------------------

#[test]
fn ord_matches_lexicographic_tuple_order() {
    let samples = [
        ts(i64::MIN, 0),
        ts(-1, u32::MAX),
        ts(0, 0),
        ts(0, 1),
        ts(1, 0),
        ts(999, u32::MAX),
        ts(1_000, 0),
        ts(1_000, 1),
        ts(i64::MAX, 0),
        ts(i64::MAX, u32::MAX),
    ];
    for a in samples {
        for b in samples {
            assert_eq!(
                a.cmp(&b),
                (a.physical_ms, a.logical).cmp(&(b.physical_ms, b.logical)),
                "{a:?} vs {b:?}"
            );
        }
    }
}

#[test]
fn new_clock_starts_at_the_zero_state() {
    let clock = Clock::new(ScriptClock::new(9_999));
    assert_eq!(clock.export_state(), State::new(ts(0, 0)));
}
