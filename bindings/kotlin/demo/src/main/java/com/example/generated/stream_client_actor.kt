package com.example.generated

import io.actrium.actr.ContextBridge
import io.actrium.actr.RpcEnvelopeBridge
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
            else -> throw io.actrium.actr.ActrException.UnknownRoute("Unknown route key: ${envelope.routeKey}")
        }
}