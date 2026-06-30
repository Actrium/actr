/**
 * Lifecycle adapter for UnifiedWorkload
 *
 * This adapter is the SDK-facing lifecycle bridge. Keep business logic in
 * [UnifiedWorkload] and generated dispatch glue under the generated package.
 */
package com.example

import io.actrium.actr.ContextBridge
import io.actrium.actr.DynamicWorkload
import io.actrium.actr.ErrorEventBridge
import io.actrium.actr.RpcEnvelopeBridge
import io.actrium.actr.WorkloadLifecycleBridge

class UnifiedLifecycleAdapter(
    private val workload: UnifiedWorkload,
) : WorkloadLifecycleBridge {
    override suspend fun onStart(ctx: ContextBridge) {
        workload.onStart(ctx)
    }

    override suspend fun onReady(ctx: ContextBridge) {
        workload.onReady(ctx)
    }

    override suspend fun onStop(ctx: ContextBridge) {
        workload.onStop(ctx)
    }

    override suspend fun onError(
        ctx: ContextBridge,
        event: ErrorEventBridge,
    ) {
        workload.onError(ctx, event)
    }

    override suspend fun dispatch(
        ctx: ContextBridge,
        envelope: RpcEnvelopeBridge,
    ): ByteArray = workload.dispatch(ctx, envelope)

    fun toDynamicWorkload(): DynamicWorkload =
        DynamicWorkload(
            lifecycle = this,
            signaling = null,
            websocket = null,
            webrtc = null,
            credential = null,
            mailbox = null,
        )
}
