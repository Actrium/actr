package {{PACKAGE_NAME}}

import android.content.Context
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import stream_server.StreamClientOuterClass.ClientStartStreamRequest
import stream_server.StreamClientOuterClass.ClientStartStreamResponse
import io.actor_rtc.actr.PayloadType
import io.actor_rtc.actr.dsl.*
import java.io.File
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * {{PROJECT_NAME_PASCAL}} DataStream Integration Test
 *
 * This test verifies the DataStream transfer to the remote StreamEchoServer.
 * The server should be running remotely before executing this test.
 *
 * Architecture:
 * ```
 * UnifiedWorkload
 *   ├── UnifiedHandler (implements StreamClientHandler)
 *   │     ├── start_stream() - locally trigger stream transfer
 *   │     └── prepare_client_stream() - server callback to register data stream receiver
 *   └── UnifiedDispatcher
 *         ├── local routes -> StreamClientDispatcher -> handler methods
 *         └── remote routes -> ctx.callRaw() -> remote actors
 * ```
 */
@RunWith(AndroidJUnit4::class)
class DataStreamIntegrationTest {

    companion object {
        private const val TAG = "DataStreamIntegrationTest"
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

    /**
     * Test DataStream transfer to remote server
     *
     * This test:
     * 1. Creates a UnifiedWorkload with MyUnifiedHandler
     * 2. Calls StartStream RPC which discovers server and sends data
     * 3. Verifies the transfer was accepted
     *
     * Prerequisites:
     * - StreamEchoServer must be running remotely
     * - Signaling server must be accessible
     */
    @Test
    fun testDataStreamTransfer(): Unit = runBlocking {
        Log.i(TAG, "=== Starting DataStream Integration Test ===")
        val clientConfigPath = copyAssetToInternalStorage("Actr.toml")
        // Actr.lock.toml is required by the runtime now
        copyAssetToInternalStorage("Actr.lock.toml")
        var clientRef: ActrRef? = null

        try {
            val clientSystem = createActrSystem(clientConfigPath)

            // Create UnifiedWorkload with handler
            val handler = MyUnifiedHandler()
            val clientWorkload = UnifiedWorkload(handler)

            val clientNode = clientSystem.attach(clientWorkload)
            clientRef = clientNode.start()
            Log.i(TAG, "Client started: ${clientRef.actorId().serialNumber}")

            // Wait for onStart to complete (auto-discover remote services)
            delay(2000)

            // ==================== Test DataStream Transfer ====================
            Log.i(TAG, "")
            Log.i(TAG, "==================== DataStream Transfer ====================")

            Log.i(TAG, "📞 Calling StartStream via UnifiedDispatcher (local service)...")
            val startStreamRequest = ClientStartStreamRequest.newBuilder()
                .setClientId("android-test-client")
                .setStreamId("test-stream-${System.currentTimeMillis()}")
                .setMessageCount(3)
                .build()

            val startStreamResponsePayload = clientRef.call(
                "stream_server.StreamClient.StartStream",
                PayloadType.RPC_RELIABLE,
                startStreamRequest.toByteArray(),
                30000L
            )

            val startStreamResponse = ClientStartStreamResponse.parseFrom(startStreamResponsePayload)
            Log.i(TAG, "📬 StartStream Response: accepted=${startStreamResponse.accepted}, message=${startStreamResponse.message}")

            assertTrue("Stream transfer should be accepted", startStreamResponse.accepted)
            Log.i(TAG, "✅ DataStream StartStream Test PASSED")

            // Wait for data stream messages to be sent (3 messages * 1 second each)
            Log.i(TAG, "⏳ Waiting for data stream messages to be sent...")
            delay(4000)

            // ==================== Summary ====================
            Log.i(TAG, "")
            Log.i(TAG, "==================== Test Summary ====================")
            Log.i(TAG, "✅ DataStream Transfer - PASSED")
            Log.i(TAG, "")
            Log.i(TAG, "=== DataStream Integration Test PASSED ===")
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
