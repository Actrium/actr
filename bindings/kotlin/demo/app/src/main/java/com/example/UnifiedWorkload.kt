/**
 * Unified Workload for all services
 *
 * This Workload handles both local and remote service requests using the UnifiedDispatcher.
 * Local requests are routed to your UnifiedHandler implementation.
 * Remote requests are forwarded to discovered remote actors.
 */
package com.example

import android.util.Log
import com.example.generated.UnifiedDispatcher
import com.example.generated.UnifiedHandler
import io.actor_rtc.actr.ActrId
import io.actor_rtc.actr.ActrType
import io.actor_rtc.actr.ContextBridge
import io.actor_rtc.actr.Realm
import io.actor_rtc.actr.RpcEnvelopeBridge
import io.actor_rtc.actr.WorkloadBridge

/**
 * Unified Workload
 *
 * Usage:
 * ```kotlin
 * val handler = MyUnifiedHandler()
 * val workload = UnifiedWorkload(handler)
 * val node = createActrNode(configPath, packagePath)
 * val actrRef = node.start()
 *
 * // Wait for remote service discovery
 * delay(2000)
 *
 * // Make local or remote RPC calls
 * val response = actrRef.call("route.key", PayloadType.RPC_RELIABLE, payload, 30000L)
 * ```
 */
class UnifiedWorkload(
    
    private val realmId: UInt = 2281844430u
) : WorkloadBridge {

    companion object {
        private const val TAG = "UnifiedWorkload"
    }

    private val selfId = ActrId(
        realm = Realm(realmId = realmId),
        serialNumber = System.currentTimeMillis().toULong(),
        type = ActrType(manufacturer = "acme", name = "UnifiedActor", version = "1.0.0")
    )

    override suspend fun onStart(ctx: ContextBridge) {
        Log.i(TAG, "UnifiedWorkload.onStart")
        // Discover all remote services
        Log.i(TAG, "📡 Discovering remote services...")
        UnifiedDispatcher.discoverRemoteServices(ctx)
        Log.i(TAG, "✅ Remote services discovered")
    }

    override suspend fun onStop(ctx: ContextBridge) {
        Log.i(TAG, "UnifiedWorkload.onStop")
    }

    /**
     * Dispatch RPC requests
     *
     * Uses the UnifiedDispatcher to route requests to:
     * - Local handler methods for local service routes
     * - Remote actors for remote service routes
     */
    override suspend fun dispatch(ctx: ContextBridge, envelope: RpcEnvelopeBridge): ByteArray {
        Log.i(TAG, "🔀 dispatch() called")
        Log.i(TAG, "   route_key: ${envelope.routeKey}")
        Log.i(TAG, "   request_id: ${envelope.requestId}")
        Log.i(TAG, "   payload size: ${envelope.payload.size} bytes")

        return UnifiedDispatcher.dispatch(ctx, envelope)
    }
}
