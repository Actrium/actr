/**
 * Lifecycle adapter for UnifiedWorkload
 *
 * This adapter is the SDK-facing lifecycle bridge. Keep business logic in
 * [UnifiedWorkload] and generated dispatch glue under the generated package.
 */
package com.example

import io.actrium.actr.dsl.ActrContext
import io.actrium.actr.dsl.DynamicWorkload
import io.actrium.actr.dsl.ErrorEvent
import io.actrium.actr.dsl.RpcEnvelope
import io.actrium.actr.dsl.Workload
import io.actrium.actr.dsl.dynamicWorkload

class UnifiedLifecycleAdapter(
    private val workload: UnifiedWorkload,
) : Workload {
    override suspend fun onStart(ctx: ActrContext) {
        workload.onStart(ctx)
    }

    override suspend fun onReady(ctx: ActrContext) {
        workload.onReady(ctx)
    }

    override suspend fun onStop(ctx: ActrContext) {
        workload.onStop(ctx)
    }

    override suspend fun onError(
        ctx: ActrContext,
        event: ErrorEvent,
    ) {
        workload.onError(ctx, event)
    }

    override suspend fun dispatch(
        ctx: ActrContext,
        envelope: RpcEnvelope,
    ): ByteArray = workload.dispatch(ctx, envelope)

    fun toDynamicWorkload(): DynamicWorkload =
        dynamicWorkload(
            lifecycle = this,
            signaling = null,
            websocket = null,
            webrtc = null,
            credential = null,
            mailbox = null,
        )
}
