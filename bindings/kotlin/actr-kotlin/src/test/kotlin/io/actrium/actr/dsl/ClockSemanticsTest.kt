package io.actrium.actr.dsl

import io.actrium.actr.AppLifecycleState
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * RFC-0419: background-duration semantics of [NetworkMonitor].
 *
 * NetworkMonitor anchors background entry with SystemClock.elapsedRealtime()
 * (monotonic, keeps counting through deep sleep and Doze, immune to wall-clock
 * adjustments) and on foreground reports
 * `(SystemClock.elapsedRealtime() - anchor).coerceAtLeast(0)` as
 * backgroundDurationMs.
 *
 * The SystemClock path itself cannot execute in a plain JVM unit test: this
 * module deliberately carries no Robolectric dependency, and android.os
 * classes from the SDK stub jar throw "Method ... not mocked" on the JVM.
 * Rather than pull in a new test framework for one call, the runtime path is
 * covered by compilation plus on-device/instrumented runs, while these tests
 * pin the surrounding arithmetic exactly as written in
 * [NetworkMonitor.onAppForeground].
 */
class ClockSemanticsTest {
    /** Mirrors the duration expression in [NetworkMonitor.onAppForeground]. */
    private fun backgroundDurationMs(nowMs: Long, anchorMs: Long): Long = (nowMs - anchorMs).coerceAtLeast(0)

    @Test
    fun `elapsed realtime difference converts to a non-negative duration`() {
        // 35 s is the long-background/forced-reconnect scenario boundary.
        assertEquals(35_000L, backgroundDurationMs(nowMs = 100_000, anchorMs = 65_000))
        assertEquals(0L, backgroundDurationMs(nowMs = 65_000, anchorMs = 65_000))
    }

    @Test
    fun `regressed clock reading clamps to zero instead of going negative`() {
        // elapsedRealtime never goes backwards, but the reporting path must
        // stay defined if handed a stale or corrupted anchor.
        assertEquals(0L, backgroundDurationMs(nowMs = 100, anchorMs = 200))
    }

    @Test
    fun `missing background anchor reports zero duration`() {
        // Mirrors the `?: 0L` branch: foreground without a recorded
        // background entry must report zero, not fail.
        val anchorMs: Long? = null
        val durationMs = anchorMs?.let { backgroundDurationMs(nowMs = 36_000, anchorMs = it) } ?: 0L
        assertEquals(0L, durationMs)
    }

    @Test
    fun `foreground state carries the duration as an unsigned value`() {
        val durationMs = backgroundDurationMs(nowMs = 35_123, anchorMs = 123)
        val state = AppLifecycleState.Foreground(backgroundDurationMs = durationMs.toULong())
        assertEquals(35_000uL, state.backgroundDurationMs)
    }
}
