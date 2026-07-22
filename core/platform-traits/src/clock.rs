//! Process-local monotonic clock abstraction (RFC-0419 §2).
//!
//! All in-process relative timing — RPC timeouts, deadlines, retry/backoff
//! intervals, heartbeat pacing, duration metrics — must be computed against a
//! monotonic clock, never against wall time (`SystemTime`, `Date.now()`,
//! protobuf timestamps): wall time can be slewed, stepped or manually changed,
//! which turns timeout arithmetic into premature or never-firing deadlines.
//!
//! [`MonotonicClock`] is that contract. Native and web runtimes provide their
//! own implementations; tests use the deterministic virtual clock `TestClock`
//! (exported from this module under the `test-utils` feature, see below).
//!
//! # Suspend semantics
//!
//! The target semantics of this abstraction is **frozen during device
//! suspend**: time spent suspended is *not* counted. A device that sleeps in
//! the middle of an RPC should not wake up to find the call timed out. Uses
//! that must measure real elapsed time across suspend (long-background
//! reconnect decisions, lease liveness windows) are a different clock class
//! entirely (suspend-aware monotonic time) and must not be built on this
//! trait.
//!
//! Implementations must pick a clock source that satisfies the freeze
//! semantics. `std::time::Instant` does **not** specify suspend behavior:
//! Linux `CLOCK_MONOTONIC` and Apple `mach_absolute_time` freeze, but Windows
//! `QueryPerformanceCounter` keeps counting through sleep — a conforming
//! Windows implementation needs a source that explicitly excludes sleep, such
//! as `QueryUnbiasedInterruptTime`.

use std::fmt::Debug;
use std::time::Duration;

/// Process-local monotonic clock for relative timing (RFC-0419 §2).
///
/// Answers exactly two questions: "how much later is this deadline?" and
/// "how long did this take?" — never "what time is it?". Readings have no
/// calendar meaning.
///
/// # Contract
///
/// Implementations and callers must uphold all of the following:
///
/// - **`elapsed` saturates.** When `later < earlier` (e.g. readings taken on
///   different cores before an OS-level clock adjustment, or caller mixed up
///   the argument order), `elapsed` returns [`Duration::ZERO`]. It never
///   panics. This matches `std::time::Instant::duration_since` semantics on
///   current std.
/// - **`add` saturates on overflow.** When `instant + duration` is not
///   representable, `add` returns an implementation-defined value that is not
///   earlier than any instant reachable from this clock — semantically a
///   deadline that never fires. It never panics. (Plain
///   `std::time::Instant + Duration` panics on overflow; implementations
///   must use `checked_add` internally.)
/// - **`Instant` values are process- and clock-local.** They must not be
///   serialized, must not be written to the wire, do not survive a process
///   restart, and must not be compared or combined across clock instances or
///   across execution contexts (e.g. a Web worker and the main thread).
///   A timeout that must survive restarts persists the business operation and
///   its policy, then recomputes a fresh local deadline after startup.
/// - **No sub-millisecond precision guarantee.** Callers must not assume
///   finer-than-millisecond resolution: browsers deliberately coarsen
///   `performance.now()` as a security mitigation.
///
/// Deadline firing semantics are uniform across runtimes: a deadline fires
/// *no earlier than* its instant; late firing is allowed and has no upper
/// bound under suspend or platform throttling (e.g. background browser tabs).
/// Expiry is guaranteed by clock semantics; the actual firing moment is a
/// scheduling property.
///
/// # Bounds
///
/// The trait requires `Send + Sync`, following the convention of the other
/// traits in this crate ([`KvStore`](crate::KvStore),
/// [`CryptoProvider`](crate::CryptoProvider),
/// [`PlatformProvider`](crate::PlatformProvider)): clock handles are shared
/// across tasks and threads on native runtimes, and wasm implementations
/// satisfy the bounds structurally. `Instant` additionally requires
/// `Send + Sync + Debug + 'static` beyond the RFC's `Copy + Ord` because
/// instants are stored inside deadline and timer state that crosses task
/// boundaries and shows up in logs and assertion messages; `Debug` output is
/// diagnostic text, not a serialization format — the non-serializability rule
/// above still applies.
pub trait MonotonicClock: Send + Sync {
    /// Opaque monotonic reading. See the trait-level contract for what it
    /// must never be used for.
    type Instant: Copy + Ord + Send + Sync + Debug + 'static;

    /// Current reading. Non-decreasing across successive calls on the same
    /// clock instance.
    fn now(&self) -> Self::Instant;

    /// `instant + duration`, for computing deadlines. Saturates to a
    /// never-firing value on overflow instead of panicking (see the
    /// trait-level contract).
    fn add(&self, instant: Self::Instant, duration: Duration) -> Self::Instant;

    /// Time elapsed from `earlier` to `later`. Returns [`Duration::ZERO`]
    /// when `later < earlier` instead of panicking (see the trait-level
    /// contract).
    fn elapsed(&self, earlier: Self::Instant, later: Self::Instant) -> Duration;
}

/// Executable contract suite for [`MonotonicClock`] implementations
/// (RFC-0419 §2; §6 R4/R5, the cross-implementation dimension).
///
/// Every implementation — the deterministic [`TestClock`], the native clock,
/// a future web clock — must pass this exact function. Running one shared
/// suite against all implementations is what makes the §2 contract portable:
/// deadline code written against the trait behaves identically on every
/// runtime. Implementing crates call this from a regular `#[test]`.
///
/// Verifies, in order:
///
/// 1. `now` is non-decreasing across successive calls;
/// 2. `add` and `elapsed` are mutually inverse for representable offsets
///    (offsets start at 1 ms: the trait explicitly guarantees no
///    sub-millisecond precision, so the shared suite must not demand it —
///    the virtual clock's nanosecond exactness is asserted in its own tests);
/// 3. `elapsed` saturates to [`Duration::ZERO`] on reversed arguments
///    instead of panicking;
/// 4. `add` saturates on overflow to a never-firing sentinel instead of
///    panicking: not earlier than its input, than any normally computed
///    deadline, or than a later reading of the clock.
///
/// The suite only advances time by calling `add` — it never sleeps — so it
/// is deterministic on virtual clocks and instantaneous on real ones.
#[cfg(any(test, feature = "test-utils"))]
pub fn assert_monotonic_clock_contract<C: MonotonicClock>(clock: &C) {
    let century = Duration::from_secs(100 * 365 * 24 * 60 * 60);

    // 1. `now` is non-decreasing.
    let mut previous = clock.now();
    for _ in 0..64 {
        let current = clock.now();
        assert!(
            current >= previous,
            "now must be non-decreasing: {previous:?} -> {current:?}"
        );
        previous = current;
    }

    // 2. `add` and `elapsed` are mutually inverse.
    let t = clock.now();
    assert_eq!(
        clock.add(t, Duration::ZERO),
        t,
        "add of zero must be identity"
    );
    for offset in [
        Duration::from_millis(1),
        Duration::from_millis(25),
        Duration::from_secs(1),
        century,
    ] {
        let later = clock.add(t, offset);
        assert!(
            later > t,
            "a positive offset must land strictly later (offset {offset:?})"
        );
        assert_eq!(
            clock.elapsed(t, later),
            offset,
            "elapsed(t, add(t, d)) must round-trip to d (offset {offset:?})"
        );
    }

    // 3. `elapsed` saturates on reversed arguments.
    let later = clock.add(t, Duration::from_secs(5));
    assert_eq!(
        clock.elapsed(later, t),
        Duration::ZERO,
        "elapsed must saturate to zero, not panic, when later < earlier"
    );
    assert_eq!(
        clock.elapsed(t, t),
        Duration::ZERO,
        "zero distance to itself"
    );

    // 4. `add` saturates on overflow to a never-firing sentinel.
    let sentinel = clock.add(t, Duration::MAX); // must not panic
    assert!(sentinel >= t, "sentinel must not precede its input instant");
    assert!(
        sentinel >= clock.add(t, century),
        "sentinel must not precede any normally computed deadline"
    );
    assert!(
        sentinel >= clock.now(),
        "sentinel must not precede a later reading of the clock"
    );
    assert!(
        clock.elapsed(t, sentinel) >= century,
        "sentinel must lie beyond any horizon a caller could wait for"
    );
}

/// Reading of a [`TestClock`]: a virtual nanosecond count.
///
/// Obtainable only through [`TestClock`]; carries no calendar meaning and,
/// like every [`MonotonicClock::Instant`], must not be serialized or compared
/// across clocks.
#[cfg(any(test, feature = "test-utils"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TestInstant(u64);

/// Deterministic virtual monotonic clock for tests (RFC-0419 §2, §6).
///
/// Time is a `u64` nanosecond counter that only moves when the test calls
/// [`advance`](TestClock::advance) — no real sleeping, no OS clock, fully
/// deterministic. The counter starts at zero and saturates at `u64::MAX`
/// (~584 years of virtual time), which doubles as the never-firing overflow
/// sentinel required by the [`MonotonicClock`] contract.
///
/// Cloning is cheap and clones share the same virtual timeline: hand a clone
/// to the code under test and keep one in the test body to drive time
/// forward. Because clones are the *same* clock instance semantically, the
/// "no cross-clock comparison" rule does not apply between them.
///
/// Gated behind `test-utils` (workspace convention for test helpers) so
/// production builds cannot depend on virtual time by accident; the `test`
/// cfg is included so this crate's own contract tests run without the
/// feature.
#[cfg(any(test, feature = "test-utils"))]
#[derive(Clone, Debug, Default)]
pub struct TestClock {
    nanos: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(any(test, feature = "test-utils"))]
impl TestClock {
    /// A clock whose virtual time starts at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Move virtual time forward by `duration`, saturating at the u64
    /// nanosecond limit. Never sleeps. Atomic: concurrent `now` callers see
    /// either the old or the new time, nothing in between.
    pub fn advance(&self, duration: Duration) {
        let delta = saturate_nanos(duration);
        self.nanos
            .fetch_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |current| Some(current.saturating_add(delta)),
            )
            .expect("fetch_update closure never returns None");
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl MonotonicClock for TestClock {
    type Instant = TestInstant;

    fn now(&self) -> TestInstant {
        TestInstant(self.nanos.load(std::sync::atomic::Ordering::SeqCst))
    }

    fn add(&self, instant: TestInstant, duration: Duration) -> TestInstant {
        // Saturation lands on u64::MAX, which is not earlier than any
        // reachable virtual instant — the never-firing sentinel the trait
        // contract requires. It is absorbing: adding to it stays put.
        TestInstant(instant.0.saturating_add(saturate_nanos(duration)))
    }

    fn elapsed(&self, earlier: TestInstant, later: TestInstant) -> Duration {
        Duration::from_nanos(later.0.saturating_sub(earlier.0))
    }
}

/// Clamp a [`Duration`] to whole u64 nanoseconds (`Duration::MAX` exceeds
/// the u64 range).
#[cfg(any(test, feature = "test-utils"))]
fn saturate_nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The minimal deadline decision the runtime performs everywhere:
    /// "has this deadline passed?". Its only notion of time is the
    /// monotonic clock — the signature admits no wall-clock input, which is
    /// the structural guarantee the wall-clock immunity test below relies
    /// on: whatever the system UTC clock does, it cannot reach this code.
    fn deadline_expired<C: MonotonicClock>(clock: &C, deadline: C::Instant) -> bool {
        clock.now() >= deadline
    }

    #[test]
    fn test_clock_upholds_the_monotonic_clock_contract() {
        // R4/R5: the virtual test clock passes the same executable contract
        // suite as every production implementation.
        assert_monotonic_clock_contract(&TestClock::new());
    }

    #[test]
    fn deadline_decisions_are_immune_to_wall_clock_jumps() {
        // R1, contract level: deadline verdicts must be a function of
        // monotonic time only. Three runs over the identical monotonic
        // schedule (checkpoints every 10 ms, deadline at 50 ms):
        //
        //   1. baseline    — no wall clock exists at all;
        //   2. monotonic   — a hostile wall clock jumps around next to the
        //                    decision; verdicts must equal the baseline;
        //   3. naive wall  — what a wall-clock-based decider would answer,
        //                    shown to diverge, proving the jump sequence is
        //                    genuinely hostile and the equality in (2) is
        //                    not vacuous.
        const STEP: Duration = Duration::from_millis(10);
        const TIMEOUT: Duration = Duration::from_millis(50);

        // Run 1: baseline, wall time does not exist.
        let baseline: Vec<bool> = {
            let clock = TestClock::new();
            let deadline = clock.add(clock.now(), TIMEOUT);
            (0..7)
                .map(|_| {
                    let verdict = deadline_expired(&clock, deadline);
                    clock.advance(STEP);
                    verdict
                })
                .collect()
        };
        assert_eq!(
            baseline,
            [false, false, false, false, false, true, true],
            "deadline at 50 ms expires exactly at the 50 ms checkpoint"
        );

        // Hostile wall-clock readings (Unix ms) observed at each checkpoint:
        // normal tick, +2 h forward jump, -1 h backward jump, a freeze held
        // for two checkpoints, then a jump back to sanity.
        const HOUR_MS: i64 = 3_600_000;
        let base_wall_ms: i64 = 1_700_000_000_000;
        let hostile_wall_ms: [i64; 7] = [
            base_wall_ms,                    // t=0    start
            base_wall_ms + 10,               // t=10   normal tick
            base_wall_ms + 2 * HOUR_MS + 20, // t=20   jumped 2 h forward
            base_wall_ms - HOUR_MS,          // t=30   jumped 1 h backward
            base_wall_ms - HOUR_MS,          // t=40   frozen
            base_wall_ms - HOUR_MS,          // t=50   still frozen (real deadline passes here)
            base_wall_ms + 60,               // t=60   back to sane
        ];

        // Run 2: same monotonic schedule with the hostile wall clock
        // alongside. `deadline_expired` cannot read `wall_ms`; the verdicts
        // must be bit-identical to the baseline.
        let clock = TestClock::new();
        let deadline = clock.add(clock.now(), TIMEOUT);
        let with_hostile_wall: Vec<bool> = hostile_wall_ms
            .iter()
            .map(|_wall_ms| {
                let verdict = deadline_expired(&clock, deadline);
                clock.advance(STEP);
                verdict
            })
            .collect();
        assert_eq!(
            with_hostile_wall, baseline,
            "monotonic deadline verdicts must not change while the wall clock jumps"
        );

        // Run 3: the naive wall-clock decider under the same jumps, to prove
        // the sequence above would actually break wall-based timing.
        let wall_deadline_ms = base_wall_ms + TIMEOUT.as_millis() as i64;
        let naive_wall: Vec<bool> = hostile_wall_ms
            .iter()
            .map(|wall_ms| *wall_ms >= wall_deadline_ms)
            .collect();
        assert_eq!(
            naive_wall,
            [false, false, true, false, false, false, true],
            "wall-based verdicts fire prematurely on the forward jump (t=20) \
             and miss the real expiry while frozen (t=50)"
        );
        assert_ne!(
            naive_wall, baseline,
            "the jump sequence must be hostile enough to break a wall-based decider"
        );
    }

    #[test]
    fn deadline_fires_no_earlier_than_its_exact_instant() {
        // R4: "no earlier than" firing semantics at its precise boundary on
        // the test runtime — 1 ns before the deadline is not expired, the
        // very next nanosecond is.
        let clock = TestClock::new();
        clock.advance(Duration::from_secs(1));
        let deadline = clock.add(clock.now(), Duration::from_millis(30));

        clock.advance(Duration::from_millis(30) - Duration::from_nanos(1));
        assert!(
            !deadline_expired(&clock, deadline),
            "1 ns before the deadline the timer must not have fired"
        );
        assert_eq!(
            clock.elapsed(deadline, clock.now()),
            Duration::ZERO,
            "no time has elapsed past a deadline that has not been reached"
        );

        clock.advance(Duration::from_nanos(1));
        assert!(
            deadline_expired(&clock, deadline),
            "at exactly the deadline instant the timer is due"
        );
        assert_eq!(
            clock.now(),
            deadline,
            "expiry happened exactly on the boundary"
        );
    }

    #[test]
    fn now_is_stable_until_advanced() {
        let clock = TestClock::new();
        assert_eq!(clock.now(), clock.now());
        assert_eq!(clock.elapsed(clock.now(), clock.now()), Duration::ZERO);
    }

    #[test]
    fn advance_moves_now_deterministically() {
        let clock = TestClock::new();
        let t0 = clock.now();

        clock.advance(Duration::from_millis(5));
        let t1 = clock.now();
        assert_eq!(clock.elapsed(t0, t1), Duration::from_millis(5));

        // Repeated advances accumulate exactly.
        clock.advance(Duration::from_millis(1));
        clock.advance(Duration::from_millis(1));
        clock.advance(Duration::from_millis(1));
        assert_eq!(clock.elapsed(t0, clock.now()), Duration::from_millis(8));

        // Zero advance is a no-op.
        clock.advance(Duration::ZERO);
        assert_eq!(clock.elapsed(t0, clock.now()), Duration::from_millis(8));
    }

    #[test]
    fn add_agrees_with_advance() {
        let clock = TestClock::new();
        clock.advance(Duration::from_secs(1));

        let deadline = clock.add(clock.now(), Duration::from_millis(7));
        assert!(
            clock.now() < deadline,
            "deadline lies in the virtual future"
        );

        clock.advance(Duration::from_millis(7));
        assert_eq!(
            clock.now(),
            deadline,
            "advancing by the same duration reaches the deadline"
        );
        assert_eq!(clock.elapsed(deadline, clock.now()), Duration::ZERO);
    }

    #[test]
    fn elapsed_saturates_to_zero_when_later_precedes_earlier() {
        let clock = TestClock::new();
        let earlier = clock.now();
        clock.advance(Duration::from_secs(3));
        let later = clock.now();

        assert_eq!(clock.elapsed(later, earlier), Duration::ZERO);
        assert_eq!(clock.elapsed(earlier, later), Duration::from_secs(3));
    }

    #[test]
    fn add_overflow_saturates_and_never_fires() {
        let clock = TestClock::new();
        clock.advance(Duration::from_secs(1));
        let t = clock.now();

        // Duration::MAX exceeds the u64 nanosecond range: must not panic.
        let sentinel = clock.add(t, Duration::MAX);

        // Not earlier than any normally reachable value.
        let far_but_normal = clock.add(t, Duration::from_secs(60 * 60 * 24 * 365));
        assert!(sentinel >= far_but_normal);
        assert!(sentinel >= t);
        clock.advance(Duration::from_secs(60 * 60 * 24 * 30));
        assert!(sentinel >= clock.now());

        // Absorbing: adding on top of the sentinel stays at the sentinel.
        assert_eq!(clock.add(sentinel, Duration::from_secs(1)), sentinel);
    }

    #[test]
    fn clones_share_the_same_timeline() {
        let clock = TestClock::new();
        let handle = clock.clone();
        let t0 = handle.now();

        clock.advance(Duration::from_millis(250));

        assert_eq!(handle.elapsed(t0, handle.now()), Duration::from_millis(250));
        assert_eq!(handle.now(), clock.now());
    }
}
