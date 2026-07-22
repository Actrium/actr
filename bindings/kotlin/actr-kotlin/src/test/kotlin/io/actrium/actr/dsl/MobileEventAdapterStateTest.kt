package io.actrium.actr.dsl

import io.actrium.actr.AppLifecycleState
import io.actrium.actr.NetworkAvailability
import io.actrium.actr.NetworkTransportFlags
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertSame
import kotlin.test.assertTrue

class MobileEventAdapterStateTest {
    @Test
    fun `lifecycle timestamps map to clamped foreground duration`() {
        val state = MobileEventAdapterState()

        assertSame(AppLifecycleState.Background, state.enterBackground(1_000))
        assertEquals(
            AppLifecycleState.Foreground(60_000uL),
            state.enterForeground(61_000),
        )
        assertEquals(
            AppLifecycleState.Foreground(0uL),
            state.enterForeground(70_000),
        )

        state.enterBackground(80_000)
        assertEquals(
            AppLifecycleState.Foreground(0uL),
            state.enterForeground(79_000),
        )
    }

    @Test
    fun `network snapshots preserve all fields and allocate monotonic sequences`() {
        val state = MobileEventAdapterState()
        val wifi =
            NetworkTransportFlags(
                wifi = true,
                cellular = false,
                ethernet = false,
                vpn = false,
                other = false,
            )
        val vpnCellular =
            NetworkTransportFlags(
                wifi = false,
                cellular = true,
                ethernet = false,
                vpn = true,
                other = false,
            )

        val first = state.snapshot(true, wifi, isExpensive = false, isConstrained = false)
        val second = state.snapshot(false, vpnCellular, isExpensive = true, isConstrained = true)

        assertEquals(1uL, first.sequence)
        assertEquals(NetworkAvailability.AVAILABLE, first.availability)
        assertTrue(first.transport.wifi)
        assertEquals(2uL, second.sequence)
        assertEquals(NetworkAvailability.UNAVAILABLE, second.availability)
        assertEquals(vpnCellular, second.transport)
        assertTrue(second.isExpensive)
        assertTrue(second.isConstrained)
    }
}
