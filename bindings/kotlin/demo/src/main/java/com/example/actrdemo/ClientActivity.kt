package com.example.actrdemo

import android.os.Bundle
import android.util.Log
import android.widget.Button
import android.widget.EditText
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import com.example.MyUnifiedHandler
import com.example.UnifiedWorkload
import data_stream_peer.StreamClientOuterClass.ClientStartStreamRequest
import data_stream_peer.StreamClientOuterClass.ClientStartStreamResponse
import echo.Echo.EchoRequest
import echo.Echo.EchoResponse
import io.actor_rtc.actr.PayloadType
import io.actor_rtc.actr.dsl.*
import io.actorrtc.demo.R
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

// NetworkMonitor is now imported from the actr-kotlin library

class ClientActivity : AppCompatActivity() {

    companion object {
        private const val TAG = "ClientActivity"
    }

    private lateinit var statusText: TextView
    private lateinit var connectButton: Button
    private lateinit var disconnectButton: Button
    private lateinit var messageInput: EditText
    private lateinit var sendButton: Button
    private lateinit var sendFileButton: Button
    private lateinit var logText: TextView
    private lateinit var scrollView: ScrollView

    // Actor-RTC components
    private var clientRef: ActrRef? = null
    private var clientSystem: ActrSystem? = null

    // Network monitoring - uses library's NetworkMonitor with automatic ActrSystem integration
    private lateinit var networkMonitor: NetworkMonitor

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_client)

        initViews()
        setupClickListeners()
        initNetworkMonitoring()

        log("Ready to connect (using UnifiedWorkload)")
    }

    private fun initNetworkMonitoring() {
        // Use the library's NetworkMonitor.create() factory method
        // It automatically integrates with ActrSystem and handles network events
        networkMonitor =
                NetworkMonitor.create(
                        context = this,
                        scope = lifecycleScope,
                        getSystem = { clientSystem }, // Lazy reference to ActrSystem
                        onNetworkStatusLog = { message ->
                            // Update UI with network status changes (runs on IO thread)
                            lifecycleScope.launch(Dispatchers.Main) { log(message) }
                        }
                )

        networkMonitor.startMonitoring()
    }

    private fun initViews() {
        statusText = findViewById(R.id.statusText)
        connectButton = findViewById(R.id.connectButton)
        disconnectButton = findViewById(R.id.disconnectButton)
        messageInput = findViewById(R.id.messageInput)
        sendButton = findViewById(R.id.sendButton)
        sendFileButton = findViewById(R.id.sendFileButton)
        logText = findViewById(R.id.logText)
        scrollView = findViewById(R.id.scrollView)
    }

    private fun setupClickListeners() {
        connectButton.setOnClickListener { connect() }

        disconnectButton.setOnClickListener { disconnect() }

        sendButton.setOnClickListener { sendMessage() }

        sendFileButton.setOnClickListener {
            // First log current network status
            val networkStatus = networkMonitor.getCurrentNetworkStatus()
            log("📡 Current network: $networkStatus")

            // Then trigger manual network check
            networkMonitor.triggerNetworkCheck()

            // Finally send file
            sendFile()
        }
    }

    private fun copyAssetToInternalStorage(assetName: String): String {
        val inputStream = assets.open(assetName)
        val outputFile = File(filesDir, assetName)
        outputFile.parentFile?.mkdirs()
        inputStream.use { input ->
            outputFile.outputStream().use { output -> input.copyTo(output) }
        }
        return outputFile.absolutePath
    }

    private fun connect() {
        updateStatus("Connecting...")
        connectButton.isEnabled = false

        lifecycleScope.launch {
            try {
                // Copy config file from assets to internal storage
                val configPath = copyAssetToInternalStorage("Actr.toml")
                copyAssetToInternalStorage("Actr.lock.toml")
                Log.i(TAG, "Config path: $configPath")

                // Create ActrSystem
                val clientSystem = createActrSystem(configPath)
                this@ClientActivity.clientSystem = clientSystem
                Log.i(TAG, "✅ ActrSystem created - NetworkMonitor will auto-handle network events")

                // Create and start UnifiedWorkload with handler
                Log.i(TAG, "🚀 Starting UnifiedWorkload...")
                val handler = MyUnifiedHandler()
                val clientWorkload = UnifiedWorkload(handler)
                val clientNode = clientSystem.attach(clientWorkload)
                clientRef = clientNode.start()
                Log.i(TAG, "✅ Client started: ${clientRef?.actorId()?.serialNumber}")

                // Wait for client to discover remote services
                delay(2000)

                withContext(Dispatchers.Main) {
                    updateStatus("Connected")
                    disconnectButton.isEnabled = true
                    messageInput.isEnabled = true
                    sendButton.isEnabled = true
                    sendFileButton.isEnabled = true
                    log("Connected (UnifiedWorkload mode)")
                    log("Client ID: ${clientRef?.actorId()?.serialNumber}")
                }
            } catch (e: Exception) {
                Log.e(TAG, "Connection failed", e)
                withContext(Dispatchers.Main) {
                    updateStatus("Connection failed")
                    connectButton.isEnabled = true
                    log("Error: ${e.message}")
                }
            }
        }
    }

    override fun onDestroy() {
        super.onDestroy()

        // Stop network monitoring
        if (::networkMonitor.isInitialized) {
            networkMonitor.stopMonitoring()
        }

        // Clean up ActrSystem
        lifecycleScope.launch {
            try {
                clientSystem?.close()
                clientSystem = null
            } catch (e: Exception) {
                Log.w(TAG, "Error during onDestroy cleanup: ${e.message}")
            }
        }
    }

    private fun disconnect() {
        updateStatus("Disconnecting...")
        disconnectButton.isEnabled = false
        messageInput.isEnabled = false
        sendButton.isEnabled = false
        sendFileButton.isEnabled = false

        lifecycleScope.launch {
            try {
                // Shutdown the client
                clientRef?.shutdown()
                clientRef?.awaitShutdown()
                clientRef = null

                // Note: clientSystem is kept alive for potential reconnection
                // NetworkMonitor will automatically handle reconnection events
                // It will be cleaned up in onDestroy()

                withContext(Dispatchers.Main) {
                    updateStatus("Disconnected")
                    connectButton.isEnabled = true
                    log("Disconnected")
                }
            } catch (e: Exception) {
                Log.e(TAG, "Disconnect error", e)
                withContext(Dispatchers.Main) {
                    updateStatus("Disconnected")
                    connectButton.isEnabled = true
                    clientRef = null
                    log("Disconnect error: ${e.message}")
                }
            }
        }
    }

    private fun sendMessage() {
        val message = messageInput.text.toString().trim()
        if (message.isEmpty()) return

        val ref = clientRef
        if (ref == null) {
            log("Error: Not connected")
            return
        }

        messageInput.text.clear()
        log("📤 Sending Echo: $message")

        lifecycleScope.launch {
            try {
                // Create EchoRequest using generated protobuf class
                val request = EchoRequest.newBuilder().setMessage(message).build()

                // Send RPC via ActrRef.call() - routed through UnifiedDispatcher
                Log.i(TAG, "📞 Sending Echo RPC via UnifiedDispatcher...")
                val responsePayload =
                        ref.call(
                                "echo.EchoService.Echo",
                                PayloadType.RPC_RELIABLE,
                                request.toByteArray(),
                                30000L
                        )

                // Parse response using generated protobuf class
                val response = EchoResponse.parseFrom(responsePayload)
                Log.i(TAG, "📬 Echo Response: ${response.reply}")

                withContext(Dispatchers.Main) { log("📥 Echo: ${response.reply}") }
            } catch (e: Exception) {
                Log.e(TAG, "Echo send error", e)
                withContext(Dispatchers.Main) { log("❌ Echo error: ${e.message}") }
            }
        }
    }

    private fun sendFile() {
        val ref = clientRef
        if (ref == null) {
            log("Error: Not connected")
            return
        }

        log("📤 Starting stream transfer...")

        lifecycleScope.launch {
            try {
                // Create ClientStartStreamRequest for local service
                val request =
                        ClientStartStreamRequest.newBuilder()
                                .setClientId("android-client")
                                .setStreamId("stream-${System.currentTimeMillis()}")
                                .setMessageCount(3)
                                .build()

                // Send RPC via ActrRef.call() - routed through UnifiedDispatcher to
                // StreamClient.StartStream
                Log.i(TAG, "📞 Sending StartStream RPC via UnifiedDispatcher (local service)...")
                val responsePayload =
                        ref.call(
                                "data_stream_peer.StreamClient.StartStream",
                                PayloadType.RPC_RELIABLE,
                                request.toByteArray(),
                                60000L
                        )

                // Parse response
                val response = ClientStartStreamResponse.parseFrom(responsePayload)
                Log.i(
                        TAG,
                        "📬 StartStream Response: accepted=${response.accepted}, message=${response.message}"
                )

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
                withContext(Dispatchers.Main) { log("❌ Stream transfer error: ${e.message}") }
            }
        }
    }

    private fun updateStatus(status: String) {
        statusText.text = "Status: $status"
    }

    private fun log(message: String) {
        val currentTime =
                java.text.SimpleDateFormat("HH:mm:ss", java.util.Locale.getDefault())
                        .format(java.util.Date())
        val logEntry = "[$currentTime] $message\n"
        logText.append(logEntry)

        // Auto scroll to bottom
        scrollView.post { scrollView.fullScroll(ScrollView.FOCUS_DOWN) }
    }
}
