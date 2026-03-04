/**
 * Auto-generated code - DO NOT EDIT
 *
 * Generated for EchoService Package: echo
 */
package com.example.actrdemo.generated

import echo.Echo.*
import io.actor_rtc.actr.ContextBridge
import io.actor_rtc.actr.RpcEnvelopeBridge

/**
 * Handler interface for EchoService
 *
 * Implement this interface to provide your business logic.
 */
interface EchoServiceHandler {
    /** RPC method: Echo */
    suspend fun echo(request: EchoRequest, ctx: ContextBridge): EchoResponse
}

/**
 * Message dispatcher for EchoService
 *
 * Routes incoming messages to the appropriate handler method.
 */
object EchoServiceDispatcher {
    /**
     * Dispatch an RPC envelope to the appropriate handler method
     *
     * @param handler The handler implementation
     * @param ctx The context bridge
     * @param envelope The RPC envelope containing the request
     * @return The serialized response bytes
     */
    suspend fun dispatch(
            handler: EchoServiceHandler,
            ctx: ContextBridge,
            envelope: RpcEnvelopeBridge
    ): ByteArray {
        return when (envelope.routeKey) {
            "echo.EchoService.Echo" -> {
                val request = EchoRequest.parseFrom(envelope.payload)
                val response = handler.echo(request, ctx)
                response.toByteArray()
            }
            else -> throw IllegalArgumentException("Unknown route key: ${envelope.routeKey}")
        }
    }
}
