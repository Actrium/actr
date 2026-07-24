package io.actrium.actr.dsl

import io.actrium.actr.CleanupReason
import io.actrium.actr.ReconnectReason
import java.util.concurrent.atomic.AtomicBoolean

internal interface NetworkMonitorLifecycle {
    fun stopMonitoring()

    fun onAppBackground()

    fun onAppForeground()

    fun cleanupConnections(reason: CleanupReason)

    fun forceReconnect(reason: ReconnectReason)

    fun triggerNetworkCheck()

    fun getCurrentNetworkStatus(): String
}

internal class MobileEventDeliveryGate {
    private val closed = AtomicBoolean(false)

    fun close() {
        closed.set(true)
    }

    suspend fun deliver(block: suspend () -> Unit) {
        if (closed.get()) return
        block()
    }
}

internal class NetworkMonitorLifecycleAdapter(
    private val monitor: NetworkMonitor,
) : NetworkMonitorLifecycle {
    override fun stopMonitoring() = monitor.stopMonitoring()

    override fun onAppBackground() = monitor.onAppBackground()

    override fun onAppForeground() = monitor.onAppForeground()

    override fun cleanupConnections(reason: CleanupReason) = monitor.cleanupConnections(reason)

    override fun forceReconnect(reason: ReconnectReason) = monitor.forceReconnect(reason)

    override fun triggerNetworkCheck() = monitor.triggerNetworkCheck()

    override fun getCurrentNetworkStatus(): String = monitor.getCurrentNetworkStatus()
}

internal class ManagedNetworkResources(
    val handle: NetworkEventHandle?,
    private val monitor: NetworkMonitorLifecycle?,
) : AutoCloseable {
    private val closed = AtomicBoolean(false)

    override fun close() {
        if (!closed.compareAndSet(false, true)) {
            return
        }

        monitor?.stopMonitoring()
        handle?.close()
    }

    fun onAppBackground() {
        if (closed.get()) return
        monitor?.onAppBackground()
    }

    fun onAppForeground() {
        if (closed.get()) return
        monitor?.onAppForeground()
    }

    fun cleanupConnections(reason: CleanupReason) {
        if (closed.get()) return
        monitor?.cleanupConnections(reason)
    }

    fun forceReconnect(reason: ReconnectReason) {
        if (closed.get()) return
        monitor?.forceReconnect(reason)
    }

    fun triggerNetworkCheck() {
        if (closed.get()) return
        monitor?.triggerNetworkCheck()
    }

    fun getCurrentNetworkStatus(): String? =
        if (closed.get()) {
            null
        } else {
            monitor?.getCurrentNetworkStatus()
        }
}
