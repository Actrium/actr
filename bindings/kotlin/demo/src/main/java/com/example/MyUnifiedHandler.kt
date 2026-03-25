/**
 * Unified Handler Implementation
 *
 * This file implements StreamClientHandler following the Rust client implementation in
 * data-stream-peer-concurrent/client.
 */
package com.example

import android.util.Log
import com.example.generated.UnifiedHandler
import data_stream_peer.DataStreamPeer.PrepareServerStreamRequest
import data_stream_peer.DataStreamPeer.PrepareStreamResponse
import data_stream_peer.StreamClientOuterClass.ClientStartStreamRequest
import data_stream_peer.StreamClientOuterClass.ClientStartStreamResponse
import data_stream_peer.StreamClientOuterClass.PrepareClientStreamRequest
import io.actor_rtc.actr.ActrId
import io.actor_rtc.actr.ActrType
import io.actor_rtc.actr.ContextBridge
import io.actor_rtc.actr.DataStream
import io.actor_rtc.actr.DataStreamCallback
import io.actor_rtc.actr.PayloadType
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Implementation of UnifiedHandler (StreamClientHandler)
 *
 * This class implements the StreamClient service following the Rust client pattern:
 * - prepare_client_stream: Called by server to prepare client for receiving data stream
 * - start_stream: Called locally to initiate a stream transfer to the server
 */
class MyUnifiedHandler : UnifiedHandler {

        companion object {
                private const val TAG = "MyUnifiedHandler"
        }

        private val serverType =
                ActrType(manufacturer = "acme", name = "DataStreamConcurrentServer", version = "1.0.0")

        // ===== StreamClient methods =====

        /**
         * PrepareClientStream - Called by the server to prepare client for receiving data stream
         *
         * This registers a DataStream handler to receive messages from the server.
         */
        override suspend fun prepare_client_stream(
                request: PrepareClientStreamRequest,
                ctx: ContextBridge
        ): PrepareStreamResponse {
                val streamId = request.streamId
                val expectedCount = request.expectedCount
                Log.i(
                        TAG,
                        "prepare_client_stream: stream_id=$streamId, expected_count=$expectedCount"
                )

                try {
                        // Register DataStream callback to receive server's data stream
                        ctx.registerStream(
                                streamId,
                                object : DataStreamCallback {
                                        override suspend fun onStream(
                                                chunk: DataStream,
                                                sender: ActrId
                                        ) {
                                                val text = String(chunk.payload, Charsets.UTF_8)
                                                Log.i(
                                                        TAG,
                                                        "client received ${chunk.sequence}/$expectedCount from ${sender.serialNumber}: $text"
                                                )
                                        }
                                }
                        )

                        Log.i(TAG, "✅ Registered stream handler for: $streamId")
                        return PrepareStreamResponse.newBuilder()
                                .setReady(true)
                                .setMessage(
                                        "client ready to receive $expectedCount messages on $streamId"
                                )
                                .build()
                } catch (e: Exception) {
                        Log.e(TAG, "❌ Failed to register stream handler: ${e.message}", e)
                        return PrepareStreamResponse.newBuilder()
                                .setReady(false)
                                .setMessage("Failed to register stream: ${e.message}")
                                .build()
                }
        }

        /**
         * StartStream - Called locally to initiate a stream transfer
         *
         * This follows the Rust client implementation:
         * 1. Discover the server
         * 2. Call PrepareServerStream RPC on the server
         * 3. Spawn a coroutine to send DataStream chunks
         */
        override suspend fun start_stream(
                request: ClientStartStreamRequest,
                ctx: ContextBridge
        ): ClientStartStreamResponse {
                val clientId = request.clientId
                val streamId = request.streamId
                val messageCount = request.messageCount

                Log.i(
                        TAG,
                        "start_stream: client_id=$clientId, stream_id=$streamId, message_count=$messageCount"
                )

                try {
                        // Discover the server
                        Log.i(
                                TAG,
                                "🌐 discovering server type: ${serverType.manufacturer}/${serverType.name}"
                        )
                        val serverId = ctx.discover(serverType)
                        Log.i(TAG, "🎯 discovered server: ${serverId.serialNumber}")

                        // Call PrepareServerStream RPC on the server
                        val prepareReq =
                                PrepareServerStreamRequest.newBuilder()
                                        .setStreamId(streamId)
                                        .setExpectedCount(messageCount)
                                        .build()

                        val prepareRespPayload =
                                ctx.callRaw(
                                        serverId,
                                        "data_stream_peer.StreamServer.PrepareStream",
                                        PayloadType.RPC_RELIABLE,
                                        prepareReq.toByteArray(),
                                        30000L
                                )
                        val prepareResp = PrepareStreamResponse.parseFrom(prepareRespPayload)

                        if (!prepareResp.ready) {
                                return ClientStartStreamResponse.newBuilder()
                                        .setAccepted(false)
                                        .setMessage(prepareResp.message)
                                        .build()
                        }

                        // Spawn a coroutine to send DataStream chunks (like tokio::spawn in Rust)
                        CoroutineScope(Dispatchers.IO).launch {
                                for (i in 1..messageCount) {
                                        val message = "[client $clientId] message $i"
                                        val dataStream =
                                                DataStream(
                                                        streamId = streamId,
                                                        sequence = i.toULong(),
                                                        payload =
                                                                message.toByteArray(Charsets.UTF_8),
                                                        metadata = emptyList(),
                                                        timestampMs = System.currentTimeMillis()
                                                )

                                        Log.i(TAG, "client sending $i/$messageCount: $message")
                                        try {
                                                ctx.sendDataStream(
                                                        serverId,
                                                        dataStream,
                                                        PayloadType.STREAM_RELIABLE
                                                )
                                        } catch (e: Exception) {
                                                Log.e(
                                                        TAG,
                                                        "client send_data_stream error: ${e.message}"
                                                )
                                        }
                                        delay(1000) // Match Rust client's 1 second delay
                                }
                        }

                        return ClientStartStreamResponse.newBuilder()
                                .setAccepted(true)
                                .setMessage(
                                        "started sending $messageCount messages to ${serverId.serialNumber}"
                                )
                                .build()
                } catch (e: Exception) {
                        Log.e(TAG, "❌ start_stream failed: ${e.message}", e)
                        return ClientStartStreamResponse.newBuilder()
                                .setAccepted(false)
                                .setMessage("Failed to start stream: ${e.message}")
                                .build()
                }
        }
}
