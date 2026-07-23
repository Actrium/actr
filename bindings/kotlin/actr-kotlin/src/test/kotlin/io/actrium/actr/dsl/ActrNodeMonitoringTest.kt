package io.actrium.actr.dsl

import io.actrium.actr.CleanupReason
import io.actrium.actr.NetworkEventHandleWrapper
import io.actrium.actr.NoHandle
import io.actrium.actr.ReconnectReason
import kotlinx.coroutines.test.runTest
import kotlin.test.Test
import kotlin.test.assertEquals

class ActrNodeMonitoringTest {
    private class RecordingHandle : NetworkEventHandleWrapper(NoHandle) {
        var closeCount = 0

        override fun close() {
            closeCount += 1
        }
    }

    private class RecordingMonitor : NetworkMonitorLifecycle {
        var stopCount = 0
        var backgroundCount = 0
        var foregroundCount = 0
        var triggerCount = 0
        val cleanupReasons = mutableListOf<CleanupReason>()
        val reconnectReasons = mutableListOf<ReconnectReason>()

        override fun stopMonitoring() {
            stopCount += 1
        }

        override fun onAppBackground() {
            backgroundCount += 1
        }

        override fun onAppForeground() {
            foregroundCount += 1
        }

        override fun cleanupConnections(reason: CleanupReason) {
            cleanupReasons += reason
        }

        override fun forceReconnect(reason: ReconnectReason) {
            reconnectReasons += reason
        }

        override fun triggerNetworkCheck() {
            triggerCount += 1
        }

        override fun getCurrentNetworkStatus(): String = "WiFi"
    }

    @Test
    fun `managed network resources close monitor and handle once`() {
        val handle = RecordingHandle()
        val monitor = RecordingMonitor()
        val resources = ManagedNetworkResources(handle, monitor)

        resources.close()
        resources.close()

        assertEquals(1, monitor.stopCount)
        assertEquals(1, handle.closeCount)
    }

    @Test
    fun `managed network resources forward lifecycle events`() {
        val monitor = RecordingMonitor()
        val resources = ManagedNetworkResources(handle = null, monitor = monitor)

        resources.onAppBackground()
        resources.onAppForeground()
        resources.cleanupConnections(CleanupReason.APP_TERMINATING)
        resources.forceReconnect(ReconnectReason.MANUAL_RECONNECT)
        resources.triggerNetworkCheck()

        assertEquals(1, monitor.backgroundCount)
        assertEquals(1, monitor.foregroundCount)
        assertEquals(listOf(CleanupReason.APP_TERMINATING), monitor.cleanupReasons)
        assertEquals(listOf(ReconnectReason.MANUAL_RECONNECT), monitor.reconnectReasons)
        assertEquals(1, monitor.triggerCount)
        assertEquals("WiFi", resources.getCurrentNetworkStatus())
    }

    @Test
    fun `managed network resources ignore stale callbacks after close`() {
        val handle = RecordingHandle()
        val monitor = RecordingMonitor()
        val resources = ManagedNetworkResources(handle, monitor)

        resources.close()
        resources.onAppBackground()
        resources.onAppForeground()
        resources.cleanupConnections(CleanupReason.MANUAL_RESET)
        resources.forceReconnect(ReconnectReason.LONG_BACKGROUND)
        resources.triggerNetworkCheck()

        assertEquals(1, monitor.stopCount)
        assertEquals(1, handle.closeCount)
        assertEquals(0, monitor.backgroundCount)
        assertEquals(0, monitor.foregroundCount)
        assertEquals(emptyList(), monitor.cleanupReasons)
        assertEquals(emptyList(), monitor.reconnectReasons)
        assertEquals(0, monitor.triggerCount)
        assertEquals(null, resources.getCurrentNetworkStatus())
    }

    @Test
    fun `mobile event delivery gate drops queued callback after close`() = runTest {
        val gate = MobileEventDeliveryGate()
        var delivered = 0

        gate.close()
        gate.deliver { delivered += 1 }

        assertEquals(0, delivered)
    }
}
