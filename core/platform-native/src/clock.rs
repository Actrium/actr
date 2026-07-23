//! Native monotonic clock, RFC-0419 §2.

use std::time::Duration;

use actr_platform_traits::MonotonicClock;

/// Native monotonic clock.
///
/// Upholds the [`MonotonicClock`] contract on every supported platform:
///
/// - `elapsed` saturates to [`Duration::ZERO`] when `later < earlier`
///   instead of panicking;
/// - `add` saturates on overflow to a never-firing, absorbing sentinel
///   instead of panicking.
///
/// # Platform backends and suspend semantics
///
/// The contract's target semantics is "frozen during device suspend": time
/// spent suspended is not counted. Each backend is chosen to satisfy it:
///
/// - **Linux / Android**: [`std::time::Instant`] (`CLOCK_MONOTONIC`), which
///   stops while the device is suspended.
/// - **Apple platforms**: [`std::time::Instant`] (`mach_absolute_time`),
///   which stops while the device is suspended.
/// - **Windows**: `QueryUnbiasedInterruptTime` via [`UnbiasedInstant`] —
///   100 ns interrupt-time ticks that explicitly exclude time spent in
///   sleep/hibernation. `std::time::Instant` is not a valid backend here:
///   it reads `QueryPerformanceCounter`, which keeps counting through
///   sleep and would fire deadlines early after a sleep/resume cycle.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeMonotonicClock;

#[cfg(not(windows))]
impl MonotonicClock for NativeMonotonicClock {
    type Instant = std::time::Instant;

    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }

    fn add(&self, instant: std::time::Instant, duration: Duration) -> std::time::Instant {
        instant.checked_add(duration).unwrap_or_else(far_future)
    }

    fn elapsed(&self, earlier: std::time::Instant, later: std::time::Instant) -> Duration {
        later.saturating_duration_since(earlier)
    }
}

/// Reading of the Windows unbiased interrupt clock: 100 ns ticks of system
/// run time, excluding time spent in sleep or hibernation.
///
/// Like every [`MonotonicClock::Instant`], values are process- and
/// clock-local: they carry no calendar meaning, must not be serialized, and
/// must not be compared across processes.
///
/// `u64::MAX` ticks (~58,000 years of run time) is the overflow sentinel of
/// [`NativeMonotonicClock::add`]: it is not earlier than any reachable
/// reading, and it is absorbing — `saturating_add` keeps every further `add`
/// at `u64::MAX`.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnbiasedInstant(u64);

/// One tick of `QueryUnbiasedInterruptTime` is 100 ns.
#[cfg(windows)]
const TICKS_PER_SECOND: u64 = 10_000_000;

#[cfg(windows)]
const NANOS_PER_TICK: u64 = 100;

/// Current unbiased interrupt time in 100 ns ticks.
#[cfg(windows)]
fn unbiased_interrupt_time_ticks() -> u64 {
    // KERNEL32 export, available since Windows 7 / Server 2008 R2 — older
    // than any Windows version the workspace supports. Declared directly:
    // the workspace has no direct windows-sys/winapi dependency (both appear
    // only transitively), and a single stable u64-out call does not justify
    // adding one.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryUnbiasedInterruptTime(unbiased_time: *mut u64) -> i32;
    }

    let mut ticks: u64 = 0;
    // SAFETY: `ticks` is a valid, writable u64 for the callee to fill in.
    let ok = unsafe { QueryUnbiasedInterruptTime(&mut ticks) };
    // The call is documented to always succeed on supported Windows versions
    // when given a valid pointer (its failure path exists only for pre-Win7
    // kernels, which cannot run this binary). If it ever failed anyway,
    // every deadline computed from a guessed reading would be wrong in an
    // unbounded way, so failing loudly beats returning fabricated time.
    assert!(ok != 0, "QueryUnbiasedInterruptTime failed");
    ticks
}

/// Convert a [`Duration`] to 100 ns ticks, rounding up: a deadline computed
/// from a sub-tick duration may only fire later than requested, never
/// earlier ("fires no earlier than" semantics). Out-of-range durations clamp
/// to `u64::MAX`, the overflow sentinel.
#[cfg(windows)]
fn duration_to_ticks_ceil(duration: Duration) -> u64 {
    let nanos = duration.as_nanos();
    let ticks = nanos / u128::from(NANOS_PER_TICK) + u128::from(!nanos.is_multiple_of(100));
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

#[cfg(windows)]
impl MonotonicClock for NativeMonotonicClock {
    type Instant = UnbiasedInstant;

    fn now(&self) -> UnbiasedInstant {
        UnbiasedInstant(unbiased_interrupt_time_ticks())
    }

    fn add(&self, instant: UnbiasedInstant, duration: Duration) -> UnbiasedInstant {
        // Saturation lands on u64::MAX, the absorbing never-firing sentinel.
        UnbiasedInstant(instant.0.saturating_add(duration_to_ticks_ceil(duration)))
    }

    fn elapsed(&self, earlier: UnbiasedInstant, later: UnbiasedInstant) -> Duration {
        let ticks = later.0.saturating_sub(earlier.0);
        // Split into whole seconds plus sub-second nanoseconds; the direct
        // `ticks * 100` nanosecond product could overflow u64.
        Duration::new(
            ticks / TICKS_PER_SECOND,
            ((ticks % TICKS_PER_SECOND) * NANOS_PER_TICK) as u32,
        )
    }
}

/// Overflow sentinel for [`NativeMonotonicClock::add`]: the largest
/// representable [`std::time::Instant`], approached to sub-nanosecond
/// distance.
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
/// Caching in a process-wide [`OnceLock`](std::sync::OnceLock) is sound
/// because every [`NativeMonotonicClock`] wraps the same OS clock; it also
/// keeps the sentinel deterministic within a process (the absorbing property
/// depends on repeated calls returning the identical value).
#[cfg(not(windows))]
fn far_future() -> std::time::Instant {
    use std::time::Instant;

    static FAR_FUTURE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
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

    /// Windows backend: sub-tick (100 ns) durations must round up, so that
    /// a deadline computed from them can only fire later than requested,
    /// never earlier.
    #[cfg(windows)]
    #[test]
    fn sub_tick_durations_round_up_never_down() {
        let clock = NativeMonotonicClock;
        let t = clock.now();

        let deadline = clock.add(t, Duration::from_nanos(1));
        assert!(deadline > t, "a 1 ns deadline must land strictly later");
        assert_eq!(
            clock.elapsed(t, deadline),
            Duration::from_nanos(100),
            "1 ns rounds up to one whole 100 ns tick"
        );

        // Exact tick multiples stay exact.
        let exact = clock.add(t, Duration::from_millis(1));
        assert_eq!(clock.elapsed(t, exact), Duration::from_millis(1));
    }
}
