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
