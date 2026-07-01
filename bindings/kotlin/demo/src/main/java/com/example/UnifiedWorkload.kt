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
import io.actrium.actr.dsl.ActrContext
import io.actrium.actr.dsl.ErrorEvent
import io.actrium.actr.dsl.RpcEnvelope

/**
 * Unified Workload lifecycle scaffold
 *
 * This handles dispatch and lifecycle-like callbacks for the linked Android client.
 * UnifiedLifecycleAdapter wraps it for the SDK-facing lifecycle bridge.
 *
 * Usage:
 * ```kotlin
 * val handler = MyUnifiedHandler()
 * val workload = UnifiedWorkload(handler)
 * val lifecycle = UnifiedLifecycleAdapter(workload)
 * val dynamicWorkload = lifecycle.toDynamicWorkload()
 * ```
 */
class UnifiedWorkload(
    private val handler: UnifiedHandler,
) {
    companion object {
        private const val TAG = "UnifiedWorkload"
    }

    suspend fun onStart(ctx: ActrContext) {
        Log.i(TAG, "UnifiedWorkload.onStart")
        // Discover all remote services
        Log.i(TAG, "📡 Discovering remote services...")
        UnifiedDispatcher.discoverRemoteServices(ctx)
        Log.i(TAG, "✅ Remote services discovered")
    }

    suspend fun onReady(ctx: ActrContext) {
        Log.i(TAG, "UnifiedWorkload.onReady")
    }

    suspend fun onStop(ctx: ActrContext) {
        Log.i(TAG, "UnifiedWorkload.onStop")
    }

    suspend fun onError(
        ctx: ActrContext,
        event: ErrorEvent,
    ) {
        Log.e(TAG, "UnifiedWorkload.onError: $event")
    }

    /**
     * Dispatch RPC requests
     *
     * Uses the UnifiedDispatcher to route requests to:
     * - Local handler methods for local service routes
     * - Remote actors for remote service routes
     */
    suspend fun dispatch(
        ctx: ActrContext,
        envelope: RpcEnvelope,
    ): ByteArray {
        Log.i(TAG, "🔀 dispatch() called")
        Log.i(TAG, "   route_key: ${envelope.routeKey}")
        Log.i(TAG, "   request_id: ${envelope.requestId}")
        Log.i(TAG, "   payload size: ${envelope.payload.size} bytes")

        return UnifiedDispatcher.dispatch(handler, ctx, envelope)
    }
}
