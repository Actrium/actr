/**
 * Unified Workload for all services (linked mode)
 *
 * This Workload handles both local and remote service requests using the UnifiedDispatcher. Local
 * requests are routed to your UnifiedHandler implementation. Remote requests are forwarded to
 * discovered remote actors.
 */
package com.example

import android.util.Log
import com.example.generated.UnifiedDispatcher
import com.example.generated.UnifiedHandler
import io.actor_rtc.actr.ActrId
import io.actor_rtc.actr.ActrType
import io.actor_rtc.actr.ContextBridge
import io.actor_rtc.actr.DynamicWorkload
import io.actor_rtc.actr.ErrorEventBridge
import io.actor_rtc.actr.Realm
import io.actor_rtc.actr.RpcEnvelopeBridge
import io.actor_rtc.actr.WorkloadLifecycleBridge

/**
 * Unified Workload lifecycle scaffold
 *
 * This can already handle dispatch and lifecycle callbacks. The generated [DynamicWorkload] wrapper
 * is ready to use once Kotlin bindings expose a linked-runtime node constructor.
 *
 * Usage:
 * ```kotlin
 * val handler = MyUnifiedHandler()
 * val workload = UnifiedWorkload(handler)
 * val dynamicWorkload = workload.toDynamicWorkload()
 * ```
 */
class UnifiedWorkload(
        private val handler: UnifiedHandler,
        private val realmId: UInt = 2281844430u
) : WorkloadLifecycleBridge {

    companion object {
        private const val TAG = "UnifiedWorkload"
    }

    private val selfId =
            ActrId(
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

    override suspend fun onReady(ctx: ContextBridge) {
        Log.i(TAG, "UnifiedWorkload.onReady")
    }

    override suspend fun onStop(ctx: ContextBridge) {
        Log.i(TAG, "UnifiedWorkload.onStop")
    }

    override suspend fun onError(ctx: ContextBridge, event: ErrorEventBridge) {
        Log.e(TAG, "UnifiedWorkload.onError: $event")
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

        return UnifiedDispatcher.dispatch(handler, ctx, envelope)
    }

    /**
     * Create a DynamicWorkload from this lifecycle scaffold.
     *
     * Kotlin bindings do not yet expose an ActrNode constructor for linked workloads, so this is
     * currently a preparation step for that API.
     */
    fun toDynamicWorkload(): DynamicWorkload {
        return DynamicWorkload(
                lifecycle = this,
                signaling = null,
                websocket = null,
                webrtc = null,
                credential = null,
                mailbox = null
        )
    }
}
