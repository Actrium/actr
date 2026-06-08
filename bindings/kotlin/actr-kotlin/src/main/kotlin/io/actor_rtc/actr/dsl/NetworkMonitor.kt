package io.actor_rtc.actr.dsl

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import android.util.Log
import io.actor_rtc.actr.NetworkAvailability
import io.actor_rtc.actr.NetworkSnapshot
import io.actor_rtc.actr.NetworkTransportFlags
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * NetworkMonitor - Network path monitor using Android ConnectivityManager.
 *
 * Monitors full network path changes (availability, transport, cost, congestion)
 * and notifies the ACTR runtime with a [NetworkSnapshot] whenever the path changes.
 *
 * Logging output examples:
 * - Network path updates with availability, transport, and cost flags
 * - Network path change events (including initial path capture)
 * - Current network status summary
 *
 * Usage:
 * 1. Create instance: NetworkMonitor.create(context, scope) { system }
 * 2. Start monitoring: startMonitoring()
 * 3. Stop monitoring: stopMonitoring()
 * 4. Notify current path: notifyCurrentPath()
 * 5. Get status: getCurrentNetworkStatus()
 *
 * Integration with ActrNode:
 * ```kotlin
 * val networkMonitor = NetworkMonitor.create(context, lifecycleScope) { system }
 * networkMonitor.startMonitoring()
 * ```
 */
class NetworkMonitor(
    private val context: Context,
    private val scope: CoroutineScope,
    private val onNetworkPathChanged: suspend (NetworkSnapshot) -> Unit,
) {
    companion object {
        private const val TAG = "NetworkMonitor"

        /**
         * Create a NetworkMonitor integrated with ActrNode.
         *
         * This factory method automatically forwards network path changes to
         * ActrNode's NetworkEventHandle, so users don't need to handle
         * network events manually.
         *
         * @param context Android Context
         * @param scope CoroutineScope, typically use lifecycleScope
         * @param getSystem Function to get ActrNode instance (may return null,
         *   e.g. before initialization)
         * @param onNetworkStatusLog Optional log callback
         * @return NetworkMonitor instance
         *
         * Example:
         * ```kotlin
         * var system: ActrNode? = null
         * val monitor = NetworkMonitor.create(this, lifecycleScope, { system }) { msg ->
         *     Log.d("App", msg)
         * }
         * monitor.startMonitoring()
         *
         * // Initialize system later
         * system = ActrNode.fromPackageFile("config.toml", "dist/app.actr")
         * ```
         */
        fun create(
            context: Context,
            scope: CoroutineScope,
            getSystem: () -> ActrNode?,
            onNetworkStatusLog: ((String) -> Unit)? = null,
        ): NetworkMonitor =
            NetworkMonitor(
                context = context,
                scope = scope,
                onNetworkPathChanged = { snapshot ->
                    handleNetworkPathChangedInternal(
                        getSystem,
                        snapshot,
                        onNetworkStatusLog,
                    )
                },
            )

        /**
         * Create a NetworkMonitor integrated with NetworkEventHandle.
         *
         * This factory method automatically forwards network path changes to
         * the specified NetworkEventHandle.
         *
         * @param context Android Context
         * @param scope CoroutineScope, typically use lifecycleScope
         * @param getHandle Function to get NetworkEventHandle instance (may
         *   return null)
         * @param onNetworkStatusLog Optional log callback
         * @return NetworkMonitor instance
         */
        fun createWithHandle(
            context: Context,
            scope: CoroutineScope,
            getHandle: () -> NetworkEventHandle?,
            onNetworkStatusLog: ((String) -> Unit)? = null,
        ): NetworkMonitor =
            NetworkMonitor(
                context = context,
                scope = scope,
                onNetworkPathChanged = { snapshot ->
                    handleNetworkPathChangedWithHandle(
                        getHandle,
                        snapshot,
                        onNetworkStatusLog,
                    )
                },
            )

        private suspend fun handleNetworkPathChangedInternal(
            getSystem: () -> ActrNode?,
            snapshot: NetworkSnapshot,
            onLog: ((String) -> Unit)?,
        ) {
            val system = getSystem()
            if (system == null) {
                Log.d(TAG, "ActrNode not available, skipping network path changed event")
                return
            }

            try {
                val handle = system.createNetworkEventHandle()
                val result =
                    handle.handleNetworkPathChangedCatching(snapshot)
                result
                    .onSuccess { eventResult ->
                        Log.i(
                            TAG,
                            "Network path changed event handled successfully: $eventResult",
                        )
                        onLog?.invoke(
                            "🌐 Network path changed - " +
                                "availability: ${snapshot.availability}, " +
                                "wifi: ${snapshot.transport.wifi}, " +
                                "cellular: ${snapshot.transport.cellular}, " +
                                "expensive: ${snapshot.isExpensive}",
                        )
                    }.onFailure { error ->
                        Log.e(
                            TAG,
                            "Failed to handle network path changed event",
                            error,
                        )
                        onLog?.invoke(
                            "❌ Network path changed event failed: ${error.message}",
                        )
                    }
            } catch (e: Exception) {
                Log.e(TAG, "Error handling network path changed", e)
                onLog?.invoke("❌ Network path changed error: ${e.message}")
            }
        }

        private suspend fun handleNetworkPathChangedWithHandle(
            getHandle: () -> NetworkEventHandle?,
            snapshot: NetworkSnapshot,
            onLog: ((String) -> Unit)?,
        ) {
            val handle = getHandle()
            if (handle == null) {
                Log.d(
                    TAG,
                    "NetworkEventHandle not available, skipping network path changed event",
                )
                return
            }

            try {
                val result =
                    handle.handleNetworkPathChangedCatching(snapshot)
                result
                    .onSuccess { eventResult ->
                        Log.i(
                            TAG,
                            "Network path changed event handled successfully: $eventResult",
                        )
                        onLog?.invoke(
                            "🌐 Network path changed - " +
                                "availability: ${snapshot.availability}, " +
                                "wifi: ${snapshot.transport.wifi}, " +
                                "cellular: ${snapshot.transport.cellular}, " +
                                "expensive: ${snapshot.isExpensive}",
                        )
                    }.onFailure { error ->
                        Log.e(
                            TAG,
                            "Failed to handle network path changed event",
                            error,
                        )
                        onLog?.invoke(
                            "❌ Network path changed event failed: ${error.message}",
                        )
                    }
            } catch (e: Exception) {
                Log.e(TAG, "Error handling network path changed", e)
                onLog?.invoke("❌ Network path changed error: ${e.message}")
            }
        }
    }

    private var connectivityManager: ConnectivityManager? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var isMonitoring = false

    // Current network path state (for deduplication)
    private var currentSnapshot: NetworkSnapshot? = null
    private var nextSequence: ULong = 1uL
    private var hasProcessedInitialPath = false

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
            Log.i(TAG, "Starting network path monitoring...")

            // Capture and log initial network path
            processCurrentPath("initial state")
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
                connectivityManager?.unregisterNetworkCallback(callback)
            }

            isMonitoring = false
            Log.i(TAG, "Stopped network monitoring")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to stop network monitoring: ${e.message}", e)
        }
    }

    /**
     * Notify the runtime of the current network path.
     *
     * This is called externally (e.g. when returning from background) to
     * re-evaluate and notify the runtime even if the path hasn't changed
     * since the last callback.
     */
    fun notifyCurrentPath() {
        scope.launch(Dispatchers.IO) {
            try {
                processCurrentPath("external notify", forceNotify = true)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to notify current path: ${e.message}", e)
            }
        }
    }

    /** Setup network callback */
    private fun setupNetworkCallback() {
        val networkRequest =
            NetworkRequest
                .Builder()
                .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
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
                    processNetworkChange(network, "onAvailable")
                }

                override fun onLost(network: Network) {
                    super.onLost(network)
                    Log.w(TAG, "Network lost: $network")
                    processCurrentPath("onLost")
                }

                override fun onCapabilitiesChanged(
                    network: Network,
                    networkCapabilities: NetworkCapabilities,
                ) {
                    super.onCapabilitiesChanged(network, networkCapabilities)
                    Log.d(
                        TAG,
                        "Network capability changed: $networkCapabilities",
                    )
                    processNetworkChange(network, "onCapabilitiesChanged")
                }
            }

        connectivityManager?.registerNetworkCallback(networkRequest, networkCallback!!)
    }

    private fun processNetworkChange(
        network: Network? = null,
        source: String = "",
    ) {
        scope.launch(Dispatchers.IO) {
            try {
                processCurrentPath(source)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to process network change ($source): ${e.message}", e)
            }
        }
    }

    /** Build and notify the current network path, deduplicating unchanged paths */
    private suspend fun processCurrentPath(
        source: String = "",
        forceNotify: Boolean = false,
    ) {
        val activeNetwork = connectivityManager?.activeNetwork
        val capabilities =
            activeNetwork?.let { connectivityManager?.getNetworkCapabilities(it) }

        val snapshot = buildSnapshot(capabilities)

        val contextStr = if (source.isNotEmpty()) " ($source)" else ""
        Log.d(
            TAG,
            "Network path$contextStr: " +
                "availability=${snapshot.availability}, " +
                "wifi=${snapshot.transport.wifi}, " +
                "cellular=${snapshot.transport.cellular}, " +
                "ethernet=${snapshot.transport.ethernet}, " +
                "vpn=${snapshot.transport.vpn}, " +
                "expensive=${snapshot.isExpensive}, " +
                "constrained=${snapshot.isConstrained}",
        )

        if (!hasProcessedInitialPath) {
            Log.i(TAG, "Network initial path captured$contextStr, forceNotify=$forceNotify")
            hasProcessedInitialPath = true
            currentSnapshot = snapshot
            if (!forceNotify) {
                return
            }
        }

        val pathChanged = forceNotify || currentSnapshot != snapshot
        if (!pathChanged) {
            return
        }

        Log.i(
            TAG,
            "Network path changed$contextStr: " +
                "sequence=${snapshot.sequence}, " +
                "availability=${snapshot.availability}",
        )
        currentSnapshot = snapshot
        onNetworkPathChanged(snapshot)
    }

    /** Build a NetworkSnapshot from NetworkCapabilities */
    private fun buildSnapshot(capabilities: NetworkCapabilities?): NetworkSnapshot {
        defer { nextSequence += 1uL }

        val transport = transportFlagsFor(capabilities)
        val availability = availabilityFor(capabilities)
        val isExpensive = !isUnmetered(capabilities)
        val isConstrained = !isUncongested(capabilities)

        return NetworkSnapshot(
            sequence = nextSequence,
            availability = availability,
            transport = transport,
            isExpensive = isExpensive,
            isConstrained = isConstrained,
        )
    }

    private fun transportFlagsFor(capabilities: NetworkCapabilities?): NetworkTransportFlags {
        if (capabilities == null) {
            return NetworkTransportFlags(
                wifi = false,
                cellular = false,
                ethernet = false,
                vpn = false,
                other = false,
            )
        }

        return NetworkTransportFlags(
            wifi = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI),
            cellular = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR),
            ethernet = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET),
            vpn = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN),
            other = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_BLUETOOTH) ||
                capabilities.hasTransport(NetworkCapabilities.TRANSPORT_LOWPAN),
        )
    }

    private fun availabilityFor(capabilities: NetworkCapabilities?): NetworkAvailability {
        if (capabilities == null) {
            return NetworkAvailability.UNAVAILABLE
        }
        return if (capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
        ) {
            NetworkAvailability.AVAILABLE
        } else {
            NetworkAvailability.UNKNOWN
        }
    }

    private fun isUnmetered(capabilities: NetworkCapabilities?): Boolean {
        if (capabilities == null) return false
        return capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
    }

    private fun isUncongested(capabilities: NetworkCapabilities?): Boolean {
        if (capabilities == null) return true
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_CONGESTED)
        } else {
            // Prior to Android 12, congestion info is not available; assume not congested
            true
        }
    }

    /** Log current network state */
    @Suppress("unused")
    private fun logCurrentNetworkState(context: String = "") {
        val activeNetwork = connectivityManager?.activeNetwork
        val capabilities = activeNetwork?.let { connectivityManager?.getNetworkCapabilities(it) }

        val networkInfo =
            if (capabilities != null) {
                val transports = mutableListOf<String>()
                if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
                    transports.add("WiFi")
                }
                if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) {
                    transports.add("Cellular")
                }
                if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) {
                    transports.add("Ethernet")
                }
                if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) {
                    transports.add("VPN")
                }

                if (transports.isNotEmpty()) {
                    transports.joinToString(", ")
                } else {
                    "no transport types"
                }
            } else {
                "no network capabilities"
            }

        val contextStr = if (context.isNotEmpty()) " ($context)" else ""
        Log.i(TAG, "Current network state$contextStr: $networkInfo")
    }

    /** Get current network status summary */
    fun getCurrentNetworkStatus(): String =
        try {
            val activeNetwork = connectivityManager?.activeNetwork
            val capabilities =
                activeNetwork?.let { connectivityManager?.getNetworkCapabilities(it) }

            when {
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true -> "VPN"
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true -> "WiFi"
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) == true ->
                    "Cellular"
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) == true ->
                    "Ethernet"
                activeNetwork != null -> "Network (unknown type)"
                else -> "No network connection"
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to get network status: ${e.message}", e)
            "Failed to get status"
        }

    /** Manually trigger network state check */
    fun triggerNetworkCheck() {
        Log.i(TAG, "Manually triggering network state check")
        scope.launch(Dispatchers.IO) {
            processCurrentPath("manual check")
        }
    }

    /** Check if currently have network connection */
    fun isConnected(): Boolean =
        currentSnapshot?.availability == NetworkAvailability.AVAILABLE

    /** Check if currently connected via WiFi */
    fun isWifi(): Boolean =
        currentSnapshot?.transport?.wifi == true

    /** Check if currently connected via mobile network */
    fun isCellular(): Boolean =
        currentSnapshot?.transport?.cellular == true

    /** Check if currently connected via VPN */
    fun isVpn(): Boolean =
        currentSnapshot?.transport?.vpn == true
}
