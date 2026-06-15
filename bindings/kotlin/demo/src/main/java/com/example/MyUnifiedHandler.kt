package com.example

import android.util.Log
import com.example.generated.UnifiedHandler
import io.actrium.actr.ActrId
import io.actrium.actr.ActrType
import io.actrium.actr.ContextBridge
import io.actrium.actr.DataStream
import io.actrium.actr.DataStreamCallback
import io.actrium.actr.PayloadType
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Implementation of UnifiedHandler (StreamClientHandler)
 *
 * Implements the DuplexStreamService client protocol:
 * 1. Call StartDuplexStream RPC on the server
 * 2. Send DataStream chunks to the server
 * 3. Optionally receive return stream from the server
 */
class MyUnifiedHandler : UnifiedHandler {
    companion object {
        private const val TAG = "MyUnifiedHandler"
    }

    private val serverType =
        ActrType(manufacturer = "actrium", name = "DuplexStreamService", version = "0.1.0")

    override suspend fun start_stream(
        request: local.StreamClientOuterClass.ClientStartStreamRequest,
        ctx: ContextBridge,
    ): local.StreamClientOuterClass.ClientStartStreamResponse {
        val clientId = request.clientId
        val sessionId = request.sessionId
        val messageCount = request.messageCount

        Log.i(
            TAG,
            "start_stream: client_id=$clientId, session_id=$sessionId, message_count=$messageCount",
        )

        try {
            Log.i(
                TAG,
                "🌐 discovering server type: ${serverType.manufacturer}/${serverType.name}",
            )
            val serverId = ctx.discover(serverType)
            Log.i(TAG, "🎯 discovered server: ${serverId.serialNumber}")

            // Step 1: Call StartDuplexStream RPC on the server
            val clientStreamId = "client-$sessionId"
            val startReq =
                local.DataStreamPeer.StartDuplexStreamRequest
                    .newBuilder()
                    .setSessionId(sessionId)
                    .setClientToServiceStreamId(clientStreamId)
                    .setClientChunkCount(messageCount.toInt())
                    .setPayloadMode(local.DataStreamPeer.StreamPayloadMode.STREAM_RELIABLE)
                    .setNote("Android duplex stream test")
                    .build()

            val startRespPayload =
                ctx.callRaw(
                    serverId,
                    "local.DuplexStreamService.StartDuplexStream",
                    PayloadType.RPC_RELIABLE,
                    startReq.toByteArray(),
                    30000L,
                )
            val startResp = local.DataStreamPeer.StartDuplexStreamResponse.parseFrom(startRespPayload)
            Log.i(
                TAG,
                "StartDuplexStream response: session=${startResp.sessionId}, " +
                    "accepted_stream=${startResp.acceptedClientToServiceStreamId}, " +
                    "return_stream=${startResp.serviceToClientStreamId}, status=${startResp.status}",
            )

            // Step 2: Register callback for the server's return stream (if any)
            val serverStreamId = startResp.serviceToClientStreamId
            if (serverStreamId.isNotBlank()) {
                ctx.registerStream(
                    serverStreamId,
                    object : DataStreamCallback {
                        override suspend fun onStream(
                            chunk: DataStream,
                            sender: ActrId,
                        ) {
                            val text = String(chunk.payload, Charsets.UTF_8)
                            Log.i(
                                TAG,
                                "client received ${chunk.sequence} from ${sender.serialNumber}: $text",
                            )
                        }
                    },
                )
                Log.i(TAG, "✅ Registered stream handler for server return stream: $serverStreamId")
            }

            // Step 3: Send DataStream chunks to the server
            CoroutineScope(Dispatchers.IO).launch {
                for (i in 1..messageCount) {
                    val message = "[client $clientId] message $i"
                    val dataStream =
                        DataStream(
                            streamId = clientStreamId,
                            sequence = i.toULong(),
                            payload = message.toByteArray(Charsets.UTF_8),
                            metadata = emptyList(),
                            timestampMs = System.currentTimeMillis(),
                        )

                    Log.i(TAG, "client sending $i/$messageCount: $message")
                    try {
                        ctx.sendDataStream(
                            serverId,
                            dataStream,
                            PayloadType.STREAM_RELIABLE,
                        )
                    } catch (e: Exception) {
                        Log.e(TAG, "client send_data_stream error: ${e.message}")
                    }
                    delay(1000)
                }

                // Step 4: Call FinishDuplexStream
                try {
                    val finishReq =
                        local.DataStreamPeer.FinishDuplexStreamRequest
                            .newBuilder()
                            .setSessionId(sessionId)
                            .setClientToServiceStreamId(clientStreamId)
                            .setServiceToClientStreamId(serverStreamId)
                            .build()
                    val finishRespPayload =
                        ctx.callRaw(
                            serverId,
                            "local.DuplexStreamService.FinishDuplexStream",
                            PayloadType.RPC_RELIABLE,
                            finishReq.toByteArray(),
                            30000L,
                        )
                    val finishResp =
                        local.DataStreamPeer.FinishDuplexStreamResponse.parseFrom(finishRespPayload)
                    Log.i(
                        TAG,
                        "FinishDuplexStream: recv=${finishResp.clientChunksReceived}, " +
                            "sent=${finishResp.serviceChunksSent}, status=${finishResp.status}",
                    )
                } catch (e: Exception) {
                    Log.e(TAG, "FinishDuplexStream error: ${e.message}")
                }
            }

            return local.StreamClientOuterClass.ClientStartStreamResponse
                .newBuilder()
                .setAccepted(true)
                .setMessage("started duplex stream: $sessionId")
                .build()
        } catch (e: Exception) {
            Log.e(TAG, "❌ start_stream failed: ${e.message}", e)
            return local.StreamClientOuterClass.ClientStartStreamResponse
                .newBuilder()
                .setAccepted(false)
                .setMessage("Failed to start stream: ${e.message}")
                .build()
        }
    }
}
