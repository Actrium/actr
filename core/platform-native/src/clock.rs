//! Native monotonic clock (`std::time::Instant`), RFC-0419 §2.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use actr_platform_traits::MonotonicClock;

/// Native monotonic clock backed by [`std::time::Instant`].
///
/// Upholds the [`MonotonicClock`] contract as follows:
///
/// - `elapsed` uses [`Instant::saturating_duration_since`], so
///   `later < earlier` yields [`Duration::ZERO`] without panicking;
/// - `add` uses [`Instant::checked_add`]; on overflow it returns a cached
///   far-future sentinel instead of panicking (see the `far_future` helper
///   in this module for the construction and its invariants).
///
/// # Suspend semantics
///
/// The contract's target semantics is "frozen during device suspend".
/// `std::time::Instant` leaves suspend behavior unspecified; concretely it
/// meets the freeze contract on Linux (`CLOCK_MONOTONIC`) and Apple platforms
/// (`mach_absolute_time`), but on Windows it is backed by
/// `QueryPerformanceCounter`, which keeps counting through sleep. A conforming
/// Windows implementation would need a source that explicitly excludes sleep
/// time, such as `QueryUnbiasedInterruptTime`; until that exists, Windows
/// deadlines may fire early relative to the freeze semantics after a
/// sleep/resume cycle. This is a documented gap, not a contract change.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeMonotonicClock;

impl MonotonicClock for NativeMonotonicClock {
    type Instant = Instant;

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn add(&self, instant: Instant, duration: Duration) -> Instant {
        instant.checked_add(duration).unwrap_or_else(far_future)
    }

    fn elapsed(&self, earlier: Instant, later: Instant) -> Duration {
        later.saturating_duration_since(earlier)
    }
}

/// Overflow sentinel for [`NativeMonotonicClock::add`]: the largest
/// representable [`Instant`], approached to sub-nanosecond distance.
///
/// `std` provides no `Instant::MAX`, so it is computed once per process by
/// binary saturation: starting from `Instant::now()`, keep adding the current
/// step while `checked_add` succeeds and halve it on failure, until the step
/// reaches zero. On failure at step `s` the remaining headroom is below `s`,
/// so each step size contributes at most two iterations and the loop
/// terminates after ~128 iterations with less than 1 ns of headroom left.
/// Instants have at most nanosecond resolution, hence no representable value
/// lies above the result.
///
/// Invariants relied on by the contract:
///
/// - every reading of the process clock and every successful `checked_add`
///   result is `<=` the sentinel, so the sentinel is never earlier than any
///   reachable instant (a deadline that never fires);
/// - the sentinel is absorbing: `add(sentinel, d)` for any `d > 0` overflows
///   and returns the sentinel again.
///
/// Caching in a process-wide [`OnceLock`] is sound because every
/// [`NativeMonotonicClock`] wraps the same OS clock; it also keeps the
/// sentinel deterministic within a process (the absorbing property depends on
/// repeated calls returning the identical value).
fn far_future() -> Instant {
    static FAR_FUTURE: OnceLock<Instant> = OnceLock::new();
    *FAR_FUTURE.get_or_init(|| {
        let mut far = Instant::now();
        let mut step = Duration::MAX;
        while step > Duration::ZERO {
            match far.checked_add(step) {
                Some(next) => far = next,
                None => step /= 2,
            }
        }
        far
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_clock_upholds_the_monotonic_clock_contract() {
        // R4/R5 cross-implementation dimension: the native clock passes the
        // same executable contract suite as the virtual TestClock, so code
        // written against the trait behaves identically on both.
        actr_platform_traits::assert_monotonic_clock_contract(&NativeMonotonicClock);
    }

    #[test]
    fn elapsed_across_a_real_wait_is_at_least_the_requested_duration() {
        // R4 on the native runtime: "fires no earlier than" against the real
        // OS clock. The blocking wait is the measurement subject itself (the
        // OS guarantees `thread::sleep` blocks for at least the requested
        // time), not a synchronization device; it is the single intentional
        // real wait in the platform clock tests and is kept short.
        let clock = NativeMonotonicClock;
        let requested = Duration::from_millis(10);

        let start = clock.now();
        std::thread::sleep(requested);
        let elapsed = clock.elapsed(start, clock.now());

        assert!(
            elapsed >= requested,
            "monotonic elapsed across a {requested:?} wait was only {elapsed:?}; \
             a deadline computed from this clock would have fired early"
        );
    }

    #[test]
    fn elapsed_saturates_to_zero_when_later_precedes_earlier() {
        let clock = NativeMonotonicClock;
        let earlier = clock.now();
        let later = clock.add(earlier, Duration::from_secs(10));

        // Reversed arguments: must saturate, not panic.
        assert_eq!(clock.elapsed(later, earlier), Duration::ZERO);
    }

    #[test]
    fn elapsed_measures_forward_distance_exactly() {
        let clock = NativeMonotonicClock;
        let t0 = clock.now();
        let t1 = clock.add(t0, Duration::from_secs(5));

        assert_eq!(clock.elapsed(t0, t1), Duration::from_secs(5));
        assert_eq!(clock.elapsed(t0, t0), Duration::ZERO);
    }

    #[test]
    fn add_zero_is_identity() {
        let clock = NativeMonotonicClock;
        let t = clock.now();
        assert_eq!(clock.add(t, Duration::ZERO), t);
    }

    #[test]
    fn add_duration_max_does_not_panic_and_never_fires() {
        let clock = NativeMonotonicClock;
        let now = clock.now();

        // `now + Duration::MAX` overflows every std Instant representation;
        // plain `+` would panic, the trait contract requires saturation.
        let sentinel = clock.add(now, Duration::MAX);

        assert!(sentinel >= now);
        // Not earlier than any normally computed deadline (100 years out).
        let century = Duration::from_secs(100 * 365 * 24 * 60 * 60);
        assert!(sentinel >= clock.add(now, century));
        // Not earlier than a later reading of the clock.
        assert!(sentinel >= clock.now());
    }

    #[test]
    fn overflow_sentinel_is_absorbing() {
        let clock = NativeMonotonicClock;
        let sentinel = clock.add(clock.now(), Duration::MAX);

        assert_eq!(clock.add(sentinel, Duration::from_secs(1)), sentinel);
        assert_eq!(clock.add(sentinel, Duration::MAX), sentinel);
    }
}
