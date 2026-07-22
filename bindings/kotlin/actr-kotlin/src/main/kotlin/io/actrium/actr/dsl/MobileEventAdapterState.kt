package io.actrium.actr.dsl

import io.actrium.actr.AppLifecycleState
import io.actrium.actr.NetworkAvailability
import io.actrium.actr.NetworkSnapshot
import io.actrium.actr.NetworkTransportFlags
import java.util.concurrent.atomic.AtomicLong

internal class MobileEventAdapterState {
    private val sequenceCounter = AtomicLong(0)

    @Volatile
    private var backgroundEnteredAtMs: Long? = null

    @Volatile
    private var lifecycleInitialized = false

    @Synchronized
    fun initializePhase(
        isForeground: Boolean,
        nowMs: Long,
    ): AppLifecycleState? {
        if (lifecycleInitialized) {
            return null
        }
        lifecycleInitialized = true
        return if (isForeground) {
            backgroundEnteredAtMs = null
            AppLifecycleState.Foreground(0uL)
        } else {
            backgroundEnteredAtMs = nowMs
            AppLifecycleState.Background
        }
    }

    @Synchronized
    fun enterBackground(nowMs: Long): AppLifecycleState {
        lifecycleInitialized = true
        backgroundEnteredAtMs = nowMs
        return AppLifecycleState.Background
    }

    @Synchronized
    fun enterForeground(nowMs: Long): AppLifecycleState {
        lifecycleInitialized = true
        val backgroundDurationMs =
            backgroundEnteredAtMs?.let { start ->
                (nowMs - start).coerceAtLeast(0)
            } ?: 0L
        backgroundEnteredAtMs = null
        return AppLifecycleState.Foreground(backgroundDurationMs.toULong())
    }

    fun snapshot(
        isAvailable: Boolean,
        transport: NetworkTransportFlags,
        isExpensive: Boolean,
        isConstrained: Boolean,
    ): NetworkSnapshot =
        NetworkSnapshot(
            sequence = sequenceCounter.incrementAndGet().toULong(),
            availability =
                if (isAvailable) {
                    NetworkAvailability.AVAILABLE
                } else {
                    NetworkAvailability.UNAVAILABLE
                },
            transport = transport,
            isExpensive = isExpensive,
            isConstrained = isConstrained,
        )
}
