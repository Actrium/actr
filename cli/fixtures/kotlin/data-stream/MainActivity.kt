package {{PACKAGE_NAME}}

import android.os.Bundle
import android.util.Log
import android.widget.Button
import android.widget.EditText
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import stream_server.StreamClientOuterClass.ClientStartStreamRequest
import stream_server.StreamClientOuterClass.ClientStartStreamResponse
import io.actor_rtc.actr.PayloadType
import io.actor_rtc.actr.dsl.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import {{PACKAGE_NAME}}.R

/**
 * {{PROJECT_NAME_PASCAL}} DataStream Client Main Activity
 *
 * This activity provides a simple UI to:
 * 1. Connect to the DataStream server via Actor-RTC
 * 2. Send data stream messages and receive responses
 * 3. Display connection status and message logs
 */
class MainActivity : AppCompatActivity() {

    companion object {
        private const val TAG = "MainActivity"
    }

    private lateinit var statusText: TextView
    private lateinit var connectButton: Button
    private lateinit var disconnectButton: Button
    private lateinit var clientIdInput: EditText
    private lateinit var messageCountInput: EditText
    private lateinit var startStreamButton: Button
    private lateinit var logText: TextView
    private lateinit var scrollView: ScrollView

    // Actor-RTC components
    private var clientRef: ActrRef? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        // Initialize UI components
        statusText = findViewById(R.id.statusText)
        connectButton = findViewById(R.id.connectButton)
        disconnectButton = findViewById(R.id.disconnectButton)
        clientIdInput = findViewById(R.id.clientIdInput)
        messageCountInput = findViewById(R.id.messageCountInput)
        startStreamButton = findViewById(R.id.startStreamButton)
        logText = findViewById(R.id.logText)
        scrollView = findViewById(R.id.scrollView)

        // Set up button listeners
        connectButton.setOnClickListener { connect() }
        disconnectButton.setOnClickListener { disconnect() }
        startStreamButton.setOnClickListener { startStream() }

        // Initial UI state
        updateConnectionState(false)
        log("Welcome to {{PROJECT_NAME_PASCAL}} DataStream Client!")
        log("Tap 'Connect' to connect to the server.")
    }

    private fun updateConnectionState(connected: Boolean) {
        connectButton.isEnabled = !connected
        disconnectButton.isEnabled = connected
        startStreamButton.isEnabled = connected
        clientIdInput.isEnabled = connected
        messageCountInput.isEnabled = connected

        if (connected) {
            updateStatus("Connected")
        } else {
            updateStatus("Disconnected")
        }
    }

    private fun connect() {
        log("🔌 Connecting to server...")
        updateStatus("Connecting...")

        lifecycleScope.launch {
            try {
                // Copy config files from assets to internal storage
                val configPath = copyAssetToInternalStorage("Actr.toml")
                copyAssetToInternalStorage("Actr.lock.toml")

                // Create ActrSystem with config
                val system = createActrSystem(configPath)

                // Create UnifiedWorkload with handler
                val handler = MyUnifiedHandler()
                val workload = UnifiedWorkload(handler)

                // Attach workload and start
                val node = system.attach(workload)
                val ref = node.start()
                clientRef = ref

                Log.i(TAG, "Client started: ${ref.actorId().serialNumber}")

                // Wait for auto-discovery to complete
                delay(2000)

                withContext(Dispatchers.Main) {
                    updateConnectionState(true)
                    log("✅ Connected successfully!")
                    log("ActorId: ${ref.actorId().serialNumber}")
                }
            } catch (e: Exception) {
                Log.e(TAG, "Connection error", e)
                withContext(Dispatchers.Main) {
                    updateConnectionState(false)
                    log("❌ Connection failed: ${e.message}")
                }
            }
        }
    }

    private fun copyAssetToInternalStorage(assetName: String): String {
        val inputStream = assets.open(assetName)
        val outputFile = File(filesDir, assetName)

        outputFile.parentFile?.mkdirs()
        inputStream.use { input ->
            outputFile.outputStream().use { output ->
                input.copyTo(output)
            }
        }
        return outputFile.absolutePath
    }

    private fun disconnect() {
        log("🔌 Disconnecting...")
        updateStatus("Disconnecting...")

        lifecycleScope.launch {
            try {
                clientRef?.shutdown()
                clientRef?.awaitShutdown()
                clientRef = null

                withContext(Dispatchers.Main) {
                    updateConnectionState(false)
                    log("✅ Disconnected successfully")
                }
            } catch (e: Exception) {
                Log.e(TAG, "Disconnect error", e)
                withContext(Dispatchers.Main) {
                    updateConnectionState(false)
                    log("❌ Disconnect error: ${e.message}")
                }
            }
        }
    }

    private fun startStream() {
        val clientId = clientIdInput.text.toString().trim().ifEmpty { "android-client" }
        val messageCount = messageCountInput.text.toString().toIntOrNull() ?: 3

        val ref = clientRef
        if (ref == null) {
            log("Error: Not connected")
            return
        }

        log("📤 Starting stream transfer...")
        log("Client ID: $clientId, Messages: $messageCount")

        lifecycleScope.launch {
            try {
                // Create ClientStartStreamRequest
                val request = ClientStartStreamRequest.newBuilder()
                    .setClientId(clientId)
                    .setStreamId("stream-${System.currentTimeMillis()}")
                    .setMessageCount(messageCount)
                    .build()

                // Send RPC via ActrRef.call() - routes to local StreamClient.StartStream
                Log.i(TAG, "📞 Sending StartStream RPC...")
                val responsePayload = ref.call(
                    "stream_server.StreamClient.StartStream",
                    PayloadType.RPC_RELIABLE,
                    request.toByteArray(),
                    60000L
                )

                // Parse response
                val response = ClientStartStreamResponse.parseFrom(responsePayload)
                Log.i(TAG, "📬 Response: accepted=${response.accepted}, message=${response.message}")
                
                withContext(Dispatchers.Main) {
                    if (response.accepted) {
                        log("✅ Stream transfer started successfully")
                        log("📝 ${response.message}")
                    } else {
                        log("❌ Stream transfer rejected: ${response.message}")
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "Stream transfer error", e)
                withContext(Dispatchers.Main) {
                    log("❌ Stream transfer error: ${e.message}")
                }
            }
        }
    }

    private fun updateStatus(status: String) {
        statusText.text = "Status: $status"
    }

    private fun log(message: String) {
        val timestamp = java.text.SimpleDateFormat("HH:mm:ss", java.util.Locale.getDefault())
            .format(java.util.Date())
        val logEntry = "[$timestamp] $message\n"
        logText.append(logEntry)
        scrollView.post { scrollView.fullScroll(ScrollView.FOCUS_DOWN) }
    }

    override fun onDestroy() {
        super.onDestroy()
        // Clean up ActrRef
        lifecycleScope.launch {
            try {
                clientRef?.shutdown()
                clientRef?.awaitShutdown()
            } catch (e: Exception) {
                Log.w(TAG, "Error during cleanup: ${e.message}")
            }
        }
    }
}
