package io.actor_rtc.actr.dsl

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * NetworkMonitor - Independent network state monitor
 *
 * Features:
 * - Monitor WiFi, mobile network, Ethernet, and VPN connection state changes
 * - Detailed logging of all network events including availability/loss and type changes
 * - Support network type change callbacks (WiFi/Cellular/VPN switching)
 * - Support network connection state change callbacks (available/unavailable)
 * - Auto-detect initial network state
 * - Support manual network state checks
 *
 * Callback descriptions:
 * - onNetworkTypeChanged: Called when network type changes (WiFi/mobile/VPN switching)
 * - onNetworkAvailable: Called when network becomes available
 * - onNetworkLost: Called when network becomes unavailable
 *
 * Logging output examples:
 * - Network available/lost events (actual connection state changes)
 * - Network capability changes (WiFi/mobile/VPN status)
 * - Network type switching events
 * - Current network status summary
 *
 * Usage:
 * 1. Create instance: NetworkMonitor(context, scope, onNetworkTypeChanged, onNetworkAvailable, onNetworkLost)
 * 2. Start monitoring: startMonitoring()
 * 3. Stop monitoring: stopMonitoring() (usually called in Activity.onDestroy)
 * 4. Manual check: triggerNetworkCheck()
 * 5. Get status: getCurrentNetworkStatus()
 *
 * Integration with ActrSystem:
 * ```kotlin
 * val networkMonitor = NetworkMonitor.create(context, lifecycleScope) { system }
 * networkMonitor.startMonitoring()
 * ```
 */
class NetworkMonitor(
        private val context: Context,
        private val scope: CoroutineScope,
        private val onNetworkTypeChanged:
                suspend (isWifi: Boolean, isCellular: Boolean, isVpn: Boolean) -> Unit =
                { _, _, _ ->
                },
        private val onNetworkAvailable: (suspend () -> Unit)? = null,
        private val onNetworkLost: (suspend () -> Unit)? = null
) {

    companion object {
        private const val TAG = "NetworkMonitor"

        /**
         * Create a NetworkMonitor integrated with ActrSystem
         *
         * This factory method automatically forwards network events to ActrSystem's NetworkEventHandle,
         * so users don't need to handle network events manually.
         *
         * @param context Android Context
         * @param scope CoroutineScope, typically use lifecycleScope
         * @param getSystem Function to get ActrSystem instance (may return null, e.g. before initialization)
         * @param onNetworkStatusLog Optional log callback to display network status changes
         * @return NetworkMonitor instance
         *
         * Example:
         * ```kotlin
         * var system: ActrSystem? = null
         * val monitor = NetworkMonitor.create(this, lifecycleScope, { system }) { msg ->
         *     Log.d("App", msg)
         * }
         * monitor.startMonitoring()
         *
         * // Initialize system later
         * system = ActrSystem.fromFile("config.toml")
         * ```
         */
        fun create(
                context: Context,
                scope: CoroutineScope,
                getSystem: () -> ActrSystem?,
                onNetworkStatusLog: ((String) -> Unit)? = null
        ): NetworkMonitor {
            return NetworkMonitor(
                    context = context,
                    scope = scope,
                    onNetworkTypeChanged = { isWifi, isCellular, isVpn ->
                        handleNetworkTypeChangedInternal(
                                getSystem,
                                isWifi,
                                isCellular,
                                isVpn,
                                onNetworkStatusLog
                        )
                    },
                    onNetworkAvailable = {
                        handleNetworkAvailableInternal(getSystem, onNetworkStatusLog)
                    },
                    onNetworkLost = { handleNetworkLostInternal(getSystem, onNetworkStatusLog) }
            )
        }

        /**
         * Create a NetworkMonitor integrated with NetworkEventHandle
         *
         * This factory method automatically forwards network events to the specified NetworkEventHandle.
         *
         * @param context Android Context
         * @param scope CoroutineScope, typically use lifecycleScope
         * @param getHandle Function to get NetworkEventHandle instance (may return null)
         * @param onNetworkStatusLog Optional log callback to display network status changes
         * @return NetworkMonitor instance
         */
        fun createWithHandle(
                context: Context,
                scope: CoroutineScope,
                getHandle: () -> NetworkEventHandle?,
                onNetworkStatusLog: ((String) -> Unit)? = null
        ): NetworkMonitor {
            return NetworkMonitor(
                    context = context,
                    scope = scope,
                    onNetworkTypeChanged = { isWifi, isCellular, isVpn ->
                        handleNetworkTypeChangedWithHandle(
                                getHandle,
                                isWifi,
                                isCellular,
                                onNetworkStatusLog
                        )
                    },
                    onNetworkAvailable = {
                        handleNetworkAvailableWithHandle(getHandle, onNetworkStatusLog)
                    },
                    onNetworkLost = { handleNetworkLostWithHandle(getHandle, onNetworkStatusLog) }
            )
        }

        private suspend fun handleNetworkTypeChangedInternal(
                getSystem: () -> ActrSystem?,
                isWifi: Boolean,
                isCellular: Boolean,
                @Suppress("UNUSED_PARAMETER") isVpn: Boolean,
                onLog: ((String) -> Unit)?
        ) {
            val system = getSystem()
            if (system == null) {
                Log.d(TAG, "ActrSystem not available, skipping network type changed event")
                return
            }

            try {
                val handle = system.createNetworkEventHandle()
                val result = handle.handleNetworkTypeChangedCatching(isWifi, isCellular)
                result
                        .onSuccess { eventResult ->
                            Log.i(
                                    TAG,
                                    "Network type changed event handled successfully: $eventResult"
                            )
                            onLog?.invoke(
                                    "🌐 Network type changed - WiFi: $isWifi, Cellular: $isCellular"
                            )
                        }
                        .onFailure { error ->
                            Log.e(TAG, "Failed to handle network type changed event", error)
                            onLog?.invoke("❌ Network type changed event failed: ${error.message}")
                        }
            } catch (e: Exception) {
                Log.e(TAG, "Error handling network type changed", e)
                onLog?.invoke("❌ Network type changed error: ${e.message}")
            }
        }

        private suspend fun handleNetworkAvailableInternal(
                getSystem: () -> ActrSystem?,
                onLog: ((String) -> Unit)?
        ) {
            val system = getSystem()
            if (system == null) {
                Log.d(TAG, "ActrSystem not available, skipping network available event")
                return
            }

            try {
                val handle = system.createNetworkEventHandle()
                val result = handle.handleNetworkAvailableCatching()
                result
                        .onSuccess { eventResult ->
                            Log.i(TAG, "Network available event handled successfully: $eventResult")
                            onLog?.invoke("🌐 Network available - handled successfully")
                        }
                        .onFailure { error ->
                            Log.e(TAG, "Failed to handle network available event", error)
                            onLog?.invoke("❌ Network available event failed: ${error.message}")
                        }
            } catch (e: Exception) {
                Log.e(TAG, "Error handling network available", e)
                onLog?.invoke("❌ Network available error: ${e.message}")
            }
        }

        private suspend fun handleNetworkLostInternal(
                getSystem: () -> ActrSystem?,
                onLog: ((String) -> Unit)?
        ) {
            val system = getSystem()
            if (system == null) {
                Log.d(TAG, "ActrSystem not available, skipping network lost event")
                return
            }

            try {
                val handle = system.createNetworkEventHandle()
                val result = handle.handleNetworkLostCatching()
                result
                        .onSuccess { eventResult ->
                            Log.i(TAG, "Network lost event handled successfully: $eventResult")
                            onLog?.invoke("🌐 Network lost - handled successfully")
                        }
                        .onFailure { error ->
                            Log.e(TAG, "Failed to handle network lost event", error)
                            onLog?.invoke("❌ Network lost event failed: ${error.message}")
                        }
            } catch (e: Exception) {
                Log.e(TAG, "Error handling network lost", e)
                onLog?.invoke("❌ Network lost error: ${e.message}")
            }
        }

        private suspend fun handleNetworkTypeChangedWithHandle(
                getHandle: () -> NetworkEventHandle?,
                isWifi: Boolean,
                isCellular: Boolean,
                onLog: ((String) -> Unit)?
        ) {
            val handle = getHandle()
            if (handle == null) {
                Log.d(TAG, "NetworkEventHandle not available, skipping network type changed event")
                return
            }

            try {
                val result = handle.handleNetworkTypeChangedCatching(isWifi, isCellular)
                result
                        .onSuccess { eventResult ->
                            Log.i(
                                    TAG,
                                    "Network type changed event handled successfully: $eventResult"
                            )
                            onLog?.invoke(
                                    "🌐 Network type changed - WiFi: $isWifi, Cellular: $isCellular"
                            )
                        }
                        .onFailure { error ->
                            Log.e(TAG, "Failed to handle network type changed event", error)
                            onLog?.invoke("❌ Network type changed event failed: ${error.message}")
                        }
            } catch (e: Exception) {
                Log.e(TAG, "Error handling network type changed", e)
                onLog?.invoke("❌ Network type changed error: ${e.message}")
            }
        }

        private suspend fun handleNetworkAvailableWithHandle(
                getHandle: () -> NetworkEventHandle?,
                onLog: ((String) -> Unit)?
        ) {
            val handle = getHandle()
            if (handle == null) {
                Log.d(TAG, "NetworkEventHandle not available, skipping network available event")
                return
            }

            try {
                val result = handle.handleNetworkAvailableCatching()
                result
                        .onSuccess { eventResult ->
                            Log.i(TAG, "Network available event handled successfully: $eventResult")
                            onLog?.invoke("🌐 Network available - handled successfully")
                        }
                        .onFailure { error ->
                            Log.e(TAG, "Failed to handle network available event", error)
                            onLog?.invoke("❌ Network available event failed: ${error.message}")
                        }
            } catch (e: Exception) {
                Log.e(TAG, "Error handling network available", e)
                onLog?.invoke("❌ Network available error: ${e.message}")
            }
        }

        private suspend fun handleNetworkLostWithHandle(
                getHandle: () -> NetworkEventHandle?,
                onLog: ((String) -> Unit)?
        ) {
            val handle = getHandle()
            if (handle == null) {
                Log.d(TAG, "NetworkEventHandle not available, skipping network lost event")
                return
            }

            try {
                val result = handle.handleNetworkLostCatching()
                result
                        .onSuccess { eventResult ->
                            Log.i(TAG, "Network lost event handled successfully: $eventResult")
                            onLog?.invoke("🌐 Network lost - handled successfully")
                        }
                        .onFailure { error ->
                            Log.e(TAG, "Failed to handle network lost event", error)
                            onLog?.invoke("❌ Network lost event failed: ${error.message}")
                        }
            } catch (e: Exception) {
                Log.e(TAG, "Error handling network lost", e)
                onLog?.invoke("❌ Network lost error: ${e.message}")
            }
        }
    }

    private var connectivityManager: ConnectivityManager? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var isMonitoring = false

    // Current network state
    private var isNetworkAvailable = false
    private var isWifiConnected = false
    private var isCellularConnected = false
    private var isVpnConnected = false

    /** Start network monitoring */
    fun startMonitoring() {
        if (isMonitoring) {
            Log.d(TAG, "Network monitoring already running")
            return
        }

        try {
            connectivityManager =
                    context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager

            setupNetworkCallback()

            isMonitoring = true
            Log.i(TAG, "Starting network state monitoring...")

            // Log initial network state
            logCurrentNetworkState("initial state")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to start network monitoring: ${e.message}", e)
        }
    }

    /** Stop network monitoring */
    fun stopMonitoring() {
        if (!isMonitoring) {
            return
        }

        try {
            networkCallback?.let { callback ->
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                    connectivityManager?.unregisterNetworkCallback(callback)
                }
            }

            isMonitoring = false
            Log.i(TAG, "Stopped network monitoring")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to stop network monitoring: ${e.message}", e)
        }
    }

    /** Setup network callback */
    private fun setupNetworkCallback() {
        val networkRequest =
                NetworkRequest.Builder()
                        .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                        // Remove capability that blocks VPN callbacks
                        .removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
                        .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                        .addTransportType(NetworkCapabilities.TRANSPORT_CELLULAR)
                        .addTransportType(NetworkCapabilities.TRANSPORT_ETHERNET)
                        .addTransportType(NetworkCapabilities.TRANSPORT_VPN)
                        .build()

        networkCallback =
                object : ConnectivityManager.NetworkCallback() {
                    override fun onAvailable(network: Network) {
                        super.onAvailable(network)
                        Log.i(TAG, "Network available: $network")

                        // Save previous network type state
                        val wasNetworkAvailable = isNetworkAvailable
                        val wasWifiConnected = isWifiConnected
                        val wasCellularConnected = isCellularConnected
                        val wasVpnConnected = isVpnConnected

                        updateNetworkState()

                        // Check if network type changed (also check in network available event)
                        if (wasWifiConnected != isWifiConnected ||
                                        wasCellularConnected != isCellularConnected ||
                                        wasVpnConnected != isVpnConnected
                        ) {
                            val networkType =
                                    when {
                                        isVpnConnected -> "VPN"
                                        isWifiConnected -> "WiFi"
                                        isCellularConnected -> "Cellular"
                                        else -> "Unknown"
                                    }

                            Log.i(
                                    TAG,
                                    "Network type changed (onAvailable): $networkType (WiFi: $isWifiConnected, Cellular: $isCellularConnected, VPN: $isVpnConnected)"
                            )

                            // Notify listener - use Dispatchers.IO to avoid blocking main thread
                            // Network event handling may require waiting for WebSocket reconnect
                            scope.launch(Dispatchers.IO) {
                                try {
                                    onNetworkTypeChanged(
                                            isWifiConnected,
                                            isCellularConnected,
                                            isVpnConnected
                                    )
                                } catch (e: Exception) {
                                    Log.e(TAG, "Failed to handle network type change: ${e.message}", e)
                                }
                            }
                        }

                        // Only trigger network available event when transitioning from unavailable to available
                        if (!wasNetworkAvailable && isNetworkAvailable) {
                            Log.i(TAG, "Network connection state changed: unavailable -> available")
                            // Use Dispatchers.IO to avoid blocking main thread
                            scope.launch(Dispatchers.IO) { onNetworkAvailable?.invoke() }
                        }
                    }

                    override fun onLost(network: Network) {
                        super.onLost(network)
                        Log.w(TAG, "Network lost: $network")

                        // Check if it's a real network connection state change
                        val wasNetworkAvailable = isNetworkAvailable
                        updateNetworkState()

                        // Only trigger network lost event when transitioning from available to unavailable
                        if (wasNetworkAvailable && !isNetworkAvailable) {
                            Log.w(TAG, "Network connection state changed: available -> unavailable")
                            // Use Dispatchers.IO to avoid blocking main thread
                            scope.launch(Dispatchers.IO) { onNetworkLost?.invoke() }
                        }
                    }

                    override fun onCapabilitiesChanged(
                            network: Network,
                            networkCapabilities: NetworkCapabilities
                    ) {
                        super.onCapabilitiesChanged(network, networkCapabilities)

                        val wasWifiConnected = isWifiConnected
                        val wasCellularConnected = isCellularConnected
                        val wasVpnConnected = isVpnConnected

                        isWifiConnected =
                                networkCapabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
                        isCellularConnected =
                                networkCapabilities.hasTransport(
                                        NetworkCapabilities.TRANSPORT_CELLULAR
                                )
                        isVpnConnected =
                                networkCapabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)

                        Log.d(
                                TAG,
                                "Network capability changed - WiFi: $isWifiConnected, Cellular: $isCellularConnected, VPN: $isVpnConnected"
                        )
                        Log.d(TAG, "Network capability details: $networkCapabilities")

                        // Check if network type changed
                        if (wasWifiConnected != isWifiConnected ||
                                        wasCellularConnected != isCellularConnected ||
                                        wasVpnConnected != isVpnConnected
                        ) {
                            val networkType =
                                    when {
                                        isVpnConnected -> "VPN"
                                        isWifiConnected -> "WiFi"
                                        isCellularConnected -> "Cellular"
                                        else -> "Unknown"
                                    }

                            Log.i(
                                    TAG,
                                    "Network type changed: $networkType (WiFi: $isWifiConnected, Cellular: $isCellularConnected, VPN: $isVpnConnected)"
                            )

                            // Notify listener - use Dispatchers.IO to avoid blocking main thread
                            // Network event handling may require waiting for WebSocket reconnect
                            scope.launch(Dispatchers.IO) {
                                try {
                                    onNetworkTypeChanged(
                                            isWifiConnected,
                                            isCellularConnected,
                                            isVpnConnected
                                    )
                                } catch (e: Exception) {
                                    Log.e(TAG, "Failed to handle network type change: ${e.message}", e)
                                }
                            }
                        }
                    }

                    override fun onLinkPropertiesChanged(
                            network: Network,
                            linkProperties: android.net.LinkProperties
                    ) {
                        super.onLinkPropertiesChanged(network, linkProperties)
                        Log.d(TAG, "Network link properties changed: $network")
                    }
                }

        connectivityManager?.registerNetworkCallback(networkRequest, networkCallback!!)
    }

    /** Update network state */
    private fun updateNetworkState() {
        val activeNetwork = connectivityManager?.activeNetwork
        val capabilities = activeNetwork?.let { connectivityManager?.getNetworkCapabilities(it) }

        val wasNetworkAvailable = isNetworkAvailable
        isNetworkAvailable = activeNetwork != null && capabilities != null

        if (capabilities != null) {
            isWifiConnected = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
            isCellularConnected = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
            isVpnConnected = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
        } else {
            isWifiConnected = false
            isCellularConnected = false
            isVpnConnected = false
        }

        val availabilityChange =
                if (wasNetworkAvailable != isNetworkAvailable) {
                    if (isNetworkAvailable) "became available" else "became unavailable"
                } else ""

        Log.d(
                TAG,
                "Network state updated - Available: $isNetworkAvailable $availabilityChange, WiFi: $isWifiConnected, Cellular: $isCellularConnected, VPN: $isVpnConnected"
        )
    }

    /** Log current network state */
    private fun logCurrentNetworkState(context: String = "") {
        val activeNetwork = connectivityManager?.activeNetwork
        val capabilities = activeNetwork?.let { connectivityManager?.getNetworkCapabilities(it) }

        val networkInfo =
                if (capabilities != null) {
                    val transports = mutableListOf<String>()
                    if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI))
                            transports.add("WiFi")
                    if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR))
                            transports.add("Cellular")
                    if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET))
                            transports.add("Ethernet")
                    if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN))
                            transports.add("VPN")

                    if (transports.isNotEmpty()) transports.joinToString(", ") else "no transport types"
                } else {
                    "no network capabilities"
                }

        val contextStr = if (context.isNotEmpty()) " ($context)" else ""
        Log.i(TAG, "Current network state$contextStr: $networkInfo")
    }

    /** Get current network status summary */
    fun getCurrentNetworkStatus(): String {
        return try {
            val activeNetwork = connectivityManager?.activeNetwork
            val capabilities =
                    activeNetwork?.let { connectivityManager?.getNetworkCapabilities(it) }

            when {
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true -> "VPN"
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true -> "WiFi"
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) == true -> "Cellular"
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) == true -> "Ethernet"
                activeNetwork != null -> "Network (unknown type)"
                else -> "No network connection"
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to get network status: ${e.message}", e)
            "Failed to get status"
        }
    }

    /** Manually trigger network state check */
    fun triggerNetworkCheck() {
        Log.i(TAG, "Manually triggering network state check")
        updateNetworkState()
        logCurrentNetworkState("manual check")
    }

    /** Check if currently have network connection */
    fun isConnected(): Boolean = isNetworkAvailable

    /** Check if currently connected via WiFi */
    fun isWifi(): Boolean = isWifiConnected

    /** Check if currently connected via mobile network */
    fun isCellular(): Boolean = isCellularConnected

    /** Check if currently connected via VPN */
    fun isVpn(): Boolean = isVpnConnected
}
