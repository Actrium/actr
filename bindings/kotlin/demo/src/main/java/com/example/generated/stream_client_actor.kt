package com.example.generated

import io.actor_rtc.actr.ContextBridge
import io.actor_rtc.actr.RpcEnvelopeBridge
import local.StreamClientOuterClass.ClientStartStreamRequest
import local.StreamClientOuterClass.ClientStartStreamResponse

interface StreamClientHandler {
    suspend fun start_stream(
        request: ClientStartStreamRequest,
        ctx: ContextBridge,
    ): ClientStartStreamResponse
}

object StreamClientDispatcher {
    suspend fun dispatch(
        handler: StreamClientHandler,
        ctx: ContextBridge,
        envelope: RpcEnvelopeBridge,
    ): ByteArray =
        when (envelope.routeKey) {
            "data_stream_peer.StreamClient.StartStream" -> {
                val request = ClientStartStreamRequest.parseFrom(envelope.payload)
                val response = handler.start_stream(request, ctx)
                response.toByteArray()
            }
            else -> throw io.actor_rtc.actr.ActrException.UnknownRoute("Unknown route key: ${envelope.routeKey}")
        }
}