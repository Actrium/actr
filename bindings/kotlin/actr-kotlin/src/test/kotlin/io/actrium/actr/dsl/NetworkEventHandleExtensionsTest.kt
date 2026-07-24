package io.actrium.actr.dsl

import io.actrium.actr.AppLifecycleState
import io.actrium.actr.CleanupReason
import io.actrium.actr.NetworkAvailability
import io.actrium.actr.NetworkEvent
import io.actrium.actr.NetworkEventHandleWrapper
import io.actrium.actr.NetworkEventResult
import io.actrium.actr.NetworkSnapshot
import io.actrium.actr.NetworkTransportFlags
import io.actrium.actr.NoHandle
import io.actrium.actr.ReconnectReason
import kotlinx.coroutines.test.runTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertSame
import kotlin.test.assertTrue

class NetworkEventHandleExtensionsTest {
    private class RecordingHandle : NetworkEventHandleWrapper(NoHandle) {
        val snapshots = mutableListOf<NetworkSnapshot>()
        val lifecycleStates = mutableListOf<AppLifecycleState>()
        val cleanupReasons = mutableListOf<CleanupReason>()
        val reconnectReasons = mutableListOf<ReconnectReason>()
        var failure: Throwable? = null

        private fun failIfRequested() {
            failure?.let { throw it }
        }

        override suspend fun handleNetworkPathChanged(snapshot: NetworkSnapshot): NetworkEventResult {
            failIfRequested()
            snapshots += snapshot
            return NetworkEventResult(NetworkEvent.NetworkPathChanged(snapshot), true, null, 1uL)
        }

        override suspend fun handleAppLifecycleChanged(state: AppLifecycleState): NetworkEventResult {
            failIfRequested()
            lifecycleStates += state
            return NetworkEventResult(NetworkEvent.AppLifecycleChanged(state), true, null, 1uL)
        }

        override suspend fun cleanupConnections(reason: CleanupReason): NetworkEventResult {
            failIfRequested()
            cleanupReasons += reason
            return NetworkEventResult(NetworkEvent.CleanupConnections(reason), true, null, 1uL)
        }

        override suspend fun forceReconnect(reason: ReconnectReason): NetworkEventResult {
            failIfRequested()
            reconnectReasons += reason
            return NetworkEventResult(NetworkEvent.ForceReconnect(reason), true, null, 1uL)
        }
    }

    private fun snapshot(): NetworkSnapshot =
        NetworkSnapshot(
            sequence = 9uL,
            availability = NetworkAvailability.AVAILABLE,
            transport = NetworkTransportFlags(true, false, false, false, false),
            isExpensive = false,
            isConstrained = true,
        )

    @Test
    fun `catching extensions forward every mobile event without altering payloads`() =
        runTest {
            val handle = RecordingHandle()
            val snapshot = snapshot()
            val foreground = AppLifecycleState.Foreground(60_000uL)

            assertTrue(handle.handleNetworkPathChangedCatching(snapshot).isSuccess)
            assertTrue(handle.handleAppLifecycleChangedCatching(foreground).isSuccess)
            assertTrue(handle.cleanupConnectionsCatching(CleanupReason.USER_LOGOUT).isSuccess)
            assertTrue(handle.forceReconnectCatching(ReconnectReason.LONG_BACKGROUND).isSuccess)

            assertEquals(listOf(snapshot), handle.snapshots)
            assertEquals(listOf<AppLifecycleState>(foreground), handle.lifecycleStates)
            assertEquals(listOf(CleanupReason.USER_LOGOUT), handle.cleanupReasons)
            assertEquals(listOf(ReconnectReason.LONG_BACKGROUND), handle.reconnectReasons)
        }

    @Test
    fun `catching extensions preserve adapter failures`() =
        runTest {
            val handle = RecordingHandle()
            val failure = IllegalStateException("adapter unavailable")
            handle.failure = failure

            val results =
                listOf(
                    handle.handleNetworkPathChangedCatching(snapshot()),
                    handle.handleAppLifecycleChangedCatching(AppLifecycleState.Background),
                    handle.cleanupConnectionsCatching(CleanupReason.MANUAL_RESET),
                    handle.forceReconnectCatching(ReconnectReason.MANUAL_RECONNECT),
                )

            assertTrue(results.all { it.isFailure })
            results.forEach { assertSame(failure, it.exceptionOrNull()) }
        }
}
