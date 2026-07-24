import Foundation
import Testing

// RFC-0419: Darwin clock semantics backing the app-lifecycle background timer.
//
// AppLifecycleMonitor (Sources/Actr/ActrNode.swift) anchors background entry
// with clock_gettime_nsec_np(CLOCK_MONOTONIC) and, on foreground, reports
// backgroundDurationMs as a saturating nanosecond difference divided by
// 1_000_000. These tests pin the clock properties that implementation relies
// on and mirror its arithmetic expression exactly.
//
// Darwin clock background:
// - CLOCK_MONOTONIC never decreases, ignores wall-clock adjustments, and
//   keeps advancing while the system is asleep.
// - CLOCK_UPTIME_RAW never decreases and ignores wall-clock adjustments, but
//   freezes while the system is asleep.
// The two clocks therefore diverge only across system sleep. Validating that
// divergence (the deep-sleep semantics) requires a physical device that
// actually enters sleep; on a simulator or an awake host these tests can only
// pin monotonicity, nanosecond units, and the awake-rate agreement below.

/// Mirrors the duration expression in
/// `AppLifecycleMonitor.handleWillEnterForeground`:
/// `nowNs >= backgroundedAtNs ? (nowNs - backgroundedAtNs) / 1_000_000 : 0`.
private func backgroundDurationMs(nowNs: UInt64, backgroundedAtNs: UInt64) -> UInt64 {
    nowNs >= backgroundedAtNs ? (nowNs - backgroundedAtNs) / 1_000_000 : 0
}

@Test func monotonicClockNeverDecreasesAcrossSamples() {
    var previous = clock_gettime_nsec_np(CLOCK_MONOTONIC)
    for _ in 0..<10_000 {
        let current = clock_gettime_nsec_np(CLOCK_MONOTONIC)
        #expect(current >= previous)
        previous = current
    }
}

@Test func monotonicClockAdvancesInNanosecondUnits() async throws {
    let t0 = clock_gettime_nsec_np(CLOCK_MONOTONIC)
    // Real elapsed time is the quantity under test here, not a
    // synchronization aid: the assertion is about how the clock encodes a
    // known wall interval.
    try await Task.sleep(nanoseconds: 200_000_000)
    let t1 = clock_gettime_nsec_np(CLOCK_MONOTONIC)

    let deltaNs = t1 - t0
    // A 200 ms wait must read as roughly 2e8. A microsecond-unit clock would
    // report ~2e5 and a millisecond-unit clock ~2e2, both far below the lower
    // bound. Task.sleep never returns early; the upper bound only allows for
    // scheduler overshoot on a loaded machine.
    #expect(deltaNs >= 150_000_000)
    #expect(deltaNs <= 20_000_000_000)
}

@Test func monotonicAndUptimeRawTickAtTheSameRateWhileAwake() async throws {
    let mono0 = clock_gettime_nsec_np(CLOCK_MONOTONIC)
    let uptime0 = clock_gettime_nsec_np(CLOCK_UPTIME_RAW)
    try await Task.sleep(nanoseconds: 1_000_000_000)
    let mono1 = clock_gettime_nsec_np(CLOCK_MONOTONIC)
    let uptime1 = clock_gettime_nsec_np(CLOCK_UPTIME_RAW)

    let monoDeltaNs = Int64(mono1 - mono0)
    let uptimeDeltaNs = Int64(uptime1 - uptime0)
    let divergenceNs = abs(monoDeltaNs - uptimeDeltaNs)

    // While the host stays awake both clocks advance at the same rate, so
    // over a one-second window their deltas agree to well under 100 ms (the
    // slack covers sampling skew between the four reads). CLOCK_MONOTONIC
    // counts time spent asleep and CLOCK_UPTIME_RAW does not, so any real
    // divergence appears only across a system sleep, which this test —
    // by design — never triggers. See the file header for the verification
    // boundary.
    #expect(divergenceNs < 100_000_000)
}

@Test func backgroundDurationConvertsNanosecondsToMilliseconds() {
    // 35 s in the background is the long-background/forced-reconnect
    // scenario boundary: 35_000_000_000 ns must convert to exactly 35_000 ms.
    #expect(backgroundDurationMs(nowNs: 35_000_000_000, backgroundedAtNs: 0) == 35_000)
    // The anchor offsets, not scales, the result.
    #expect(backgroundDurationMs(nowNs: 5_000_000_000 + 123, backgroundedAtNs: 123) == 5_000)
    // Integer division truncates toward zero.
    #expect(backgroundDurationMs(nowNs: 1_999_999, backgroundedAtNs: 0) == 1)
    #expect(backgroundDurationMs(nowNs: 999_999, backgroundedAtNs: 0) == 0)
}

@Test func backgroundDurationSaturatesInsteadOfTrapping() {
    // nowNs < backgroundedAtNs would underflow UInt64 subtraction and trap;
    // the guarded expression must yield 0 instead.
    #expect(backgroundDurationMs(nowNs: 0, backgroundedAtNs: 1) == 0)
    #expect(backgroundDurationMs(nowNs: 41, backgroundedAtNs: UInt64.max) == 0)
    // A zero-length window is 0 ms, and an extreme forward distance stays
    // defined without overflow.
    #expect(backgroundDurationMs(nowNs: 7, backgroundedAtNs: 7) == 0)
    #expect(backgroundDurationMs(nowNs: UInt64.max, backgroundedAtNs: 0) == UInt64.max / 1_000_000)
}

@Test func measuredBackgroundWindowProducesPlausibleMilliseconds() async throws {
    // End-to-end shape of the production path: anchor, dwell, convert.
    let anchor = clock_gettime_nsec_np(CLOCK_MONOTONIC)
    try await Task.sleep(nanoseconds: 50_000_000)
    let now = clock_gettime_nsec_np(CLOCK_MONOTONIC)

    let durationMs = backgroundDurationMs(nowNs: now, backgroundedAtNs: anchor)
    #expect(durationMs >= 40)
    #expect(durationMs <= 20_000)
}
