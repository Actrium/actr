/**
 * 统一集成测试
 *
 * 此测试使用 UnifiedWorkload 模式，合并了以下两个测试用例：
 * 1. RPC 调用 EchoServer
 * 2. DataStream 传输（通过 StreamClient.StartStream）
 *
 * UnifiedWorkload 的优势：
 * - 本地服务请求：通过 UnifiedDispatcher 路由到 StreamClientHandler 实现
 * - 远程服务请求：通过 UnifiedDispatcher 自动转发到已发现的远程 Actor
 * - 统一的 onStart 中自动发现所有远程服务
 */
package com.example.actrdemo

import android.content.Context
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.example.MyUnifiedHandler
import com.example.UnifiedWorkload
import data_stream_peer.StreamClientOuterClass.ClientStartStreamRequest
import data_stream_peer.StreamClientOuterClass.ClientStartStreamResponse
import io.actor_rtc.actr.PayloadType
import io.actor_rtc.actr.dsl.*
import java.io.File
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class UnifiedIntegrationTest {

    companion object {
        private const val TAG = "UnifiedIntegrationTest"
    }

    private fun getContext(): Context {
        return InstrumentationRegistry.getInstrumentation().targetContext
    }

    private fun copyAssetToInternalStorage(assetName: String): String {
        // Source: Test Assets (src/androidTest/assets)
        val sourceContext = InstrumentationRegistry.getInstrumentation().context
        val inputStream = sourceContext.assets.open(assetName)

        // Destination: App Files Dir (standard app storage)
        val destContext = InstrumentationRegistry.getInstrumentation().targetContext
        val outputFile = File(destContext.filesDir, assetName)

        outputFile.parentFile?.mkdirs()
        inputStream.use { input ->
            outputFile.outputStream().use { output -> input.copyTo(output) }
        }
        return outputFile.absolutePath
    }

    // ==================== Protobuf Encoding/Decoding Helpers ====================

    private fun encodeEchoRequest(message: String): ByteArray {
        val messageBytes = message.toByteArray(Charsets.UTF_8)
        val result = ByteArray(2 + messageBytes.size)
        result[0] = 0x0a.toByte()
        result[1] = messageBytes.size.toByte()
        System.arraycopy(messageBytes, 0, result, 2, messageBytes.size)
        return result
    }

    private fun decodeEchoResponse(payload: ByteArray): String {
        if (payload.isEmpty()) return ""
        var offset = 0
        while (offset < payload.size) {
            val tag = payload[offset].toInt() and 0xFF
            offset++
            val fieldNumber = tag shr 3
            val wireType = tag and 0x07
            if (fieldNumber == 1 && wireType == 2) {
                val length = payload[offset].toInt() and 0xFF
                offset++
                return String(payload, offset, length, Charsets.UTF_8)
            }
            break
        }
        return ""
    }

    // ==================== Unified Integration Test ====================

    /**
     * 统一集成测试
     *
     * 此测试使用 UnifiedWorkload 同时验证：
     * 1. 远程 RPC 调用 (EchoService)
     * 2. DataStream 传输 (StreamClient.StartStream -> DataStreamConcurrentServer)
     *
     * 架构：
     * ```
     * UnifiedWorkload
     *   ├── UnifiedHandler (implements StreamClientHandler)
     *   │     ├── start_stream() - 本地触发流传输
     *   │     └── prepare_client_stream() - 服务器回调注册数据流接收器
     *   └── UnifiedDispatcher
     *         ├── local routes -> StreamClientDispatcher -> handler methods
     *         └── remote routes -> ctx.callRaw() -> remote actors
     * ```
     */
    @Test
    fun testUnifiedWorkloadWithEchoAndDataStream(): Unit = runBlocking {
        Log.i(TAG, "=== Starting Unified Integration Test ===")
        Log.i(TAG, "This test combines Echo RPC and DataStream transfer")
        val clientConfigPath = copyAssetToInternalStorage("Actr.toml")
        // Actr.lock.toml is required by the runtime now
        copyAssetToInternalStorage("Actr.lock.toml")
        var clientRef: ActrRef? = null

        try {
            val clientSystem = createActrSystem(clientConfigPath)

            // 创建 UnifiedWorkload
            val handler = MyUnifiedHandler()
            val clientWorkload = UnifiedWorkload(handler)

            val clientNode = clientSystem.attach(clientWorkload)
            clientRef = clientNode.start()
            Log.i(TAG, "Client started: ${clientRef.actorId().serialNumber}")

            // 等待 onStart 完成（自动发现所有远程服务）
            delay(2000)

            // ==================== Part 1: Test Echo RPC ====================
            Log.i(TAG, "")
            Log.i(TAG, "==================== Part 1: Echo RPC ====================")
            val testMessage = "Hello from Android Unified Test!"
            val expectedResponse = "Echo: $testMessage"

            Log.i(TAG, "📞 Sending RPC to EchoService via UnifiedDispatcher...")
            val echoRequestPayload = encodeEchoRequest(testMessage)

            val echoResponsePayload =
                    clientRef.call(
                            "echo.EchoService.Echo",
                            PayloadType.RPC_RELIABLE,
                            echoRequestPayload,
                            30000L
                    )

            val echoResponse = decodeEchoResponse(echoResponsePayload)
            Log.i(TAG, "📬 Echo Response: $echoResponse")

            assertEquals("Echo mismatch", expectedResponse, echoResponse)
            Log.i(TAG, "✅ Echo RPC Test PASSED")

            // ==================== Part 2: Test DataStream Transfer ====================
            Log.i(TAG, "")
            Log.i(TAG, "==================== Part 2: DataStream Transfer ====================")

            Log.i(TAG, "📞 Calling StartStream via UnifiedDispatcher (local service)...")
            val startStreamRequest =
                    ClientStartStreamRequest.newBuilder()
                            .setClientId("android-test-client")
                            .setStreamId("test-stream-${System.currentTimeMillis()}")
                            .setMessageCount(3)
                            .build()

            val startStreamResponsePayload =
                    clientRef.call(
                            "data_stream_peer.StreamClient.StartStream",
                            PayloadType.RPC_RELIABLE,
                            startStreamRequest.toByteArray(),
                            30000L
                    )

            val startStreamResponse =
                    ClientStartStreamResponse.parseFrom(startStreamResponsePayload)
            Log.i(
                    TAG,
                    "📬 StartStream Response: accepted=${startStreamResponse.accepted}, message=${startStreamResponse.message}"
            )

            assertTrue("Stream transfer should be accepted", startStreamResponse.accepted)
            Log.i(TAG, "✅ DataStream StartStream Test PASSED")

            // Wait for data stream messages to be sent (3 messages * 1 second each)
            Log.i(TAG, "⏳ Waiting for data stream messages to be sent...")
            delay(4000)

            // ==================== Summary ====================
            Log.i(TAG, "")
            Log.i(TAG, "==================== Test Summary ====================")
            Log.i(TAG, "✅ Part 1: Echo RPC - PASSED")
            Log.i(TAG, "✅ Part 2: DataStream Transfer - PASSED")
            Log.i(TAG, "")
            Log.i(TAG, "=== Unified Integration Test PASSED ===")
        } finally {
            try {
                clientRef?.shutdown()
                clientRef?.awaitShutdown()
            } catch (e: Exception) {
                Log.w(TAG, "Error during shutdown: ${e.message}")
            }
        }
    }
}
