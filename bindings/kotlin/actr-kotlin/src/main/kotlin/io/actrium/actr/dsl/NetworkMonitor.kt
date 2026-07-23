package io.actrium.actr.dsl

import android.app.ActivityManager
import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import android.util.Log
import io.actrium.actr.ActrException
import io.actrium.actr.AppLifecycleState
import io.actrium.actr.CleanupReason
import io.actrium.actr.NetworkSnapshot
import io.actrium.actr.NetworkTransportFlags
import io.actrium.actr.ReconnectReason
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch

/**
 * NetworkMonitor - Independent network state monitor
 *
 * Features:
 * - Monitor WiFi, mobile network, Ethernet, and VPN connection state changes
 * - Builds full NetworkSnapshot from Android system APIs and reports via
 *   handleNetworkPathChanged
 * - Supports app lifecycle callbacks (background/foreground) via
 *   handleAppLifecycleChanged
 * - Supports cleanupConnections and forceReconnect for explicit lifecycle ops
 * - Auto-detect initial network state
 * - Injects one authoritative initial app lifecycle state
 * - Support manual network state checks
 *
 * Logging output examples:
 * - Network path change events (full NetworkSnapshot)
 * - App lifecycle change events (background/foreground with duration)
 * - Current network status summary
 *
 * Usage:
 * 1. Create instance: NetworkMonitor(context, scope, onNetworkPathChanged, onAppLifecycleChanged)
 * 2. Start monitoring: startMonitoring()
 * 3. Stop monitoring: stopMonitoring() (usually called in Activity.onDestroy)
 * 4. Manual check: triggerNetworkCheck()
 * 5. Get status: getCurrentNetworkStatus()
 * 6. Cleanup/reconnect: cleanupConnections(), forceReconnect()
 *
 * Integration with ActrNode:
 * ```kotlin
 * val networkMonitor = NetworkMonitor.create(context, lifecycleScope) { system }
 * networkMonitor.startMonitoring()
 * ```
 *
 * On foreground return, report background_duration_ms to the Rust runtime,
 * which decides whether to connection-probe or force-reconnect.
 */
class NetworkMonitor
    (
    private val context: Context,
    private val scope: CoroutineScope,
    private val onNetworkPathChanged: (suspend (NetworkSnapshot) -> Unit)? = null,
    private val onAppLifecycleChanged: (suspend (AppLifecycleState) -> Unit)? = null,
    private val onCleanupConnections: (suspend (CleanupReason) -> Unit)? = null,
    private val onForceReconnect: (suspend (ReconnectReason) -> Unit)? = null,
) {
    companion object {
        private const val TAG = "NetworkMonitor"

        /**
         * Create a NetworkMonitor integrated with ActrNode
         *
         * This factory method automatically forwards network events to ActrNode's
         * NetworkEventHandle, so users don't need to handle network events manually.
         *
         * @param context Android Context
         * @param scope CoroutineScope, typically use lifecycleScope
         * @param getSystem Function to get ActrNode instance (may return null, e.g. before initialization)
         * @param onNetworkStatusLog Optional log callback to display network status changes
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
                    handleNetworkPathChangedInternal(getSystem, snapshot, onNetworkStatusLog)
                },
                onAppLifecycleChanged = { state ->
                    handleAppLifecycleChangedInternal(getSystem, state, onNetworkStatusLog)
                },
                onCleanupConnections = { reason ->
                    handleCleanupConnectionsInternal(getSystem, reason, onNetworkStatusLog)
                },
                onForceReconnect = { reason ->
                    handleForceReconnectInternal(getSystem, reason, onNetworkStatusLog)
                },
            )

        /**
         * Create a NetworkMonitor integrated with NetworkEventHandle
         *
         * This factory method automatically forwards network events to the specified
         * NetworkEventHandle.
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
            onNetworkStatusLog: ((String) -> Unit)? = null,
        ): NetworkMonitor =
            NetworkMonitor(
                context = context,
                scope = scope,
                onNetworkPathChanged = { snapshot ->
                    handleNetworkPathChangedWithHandle(getHandle, snapshot, onNetworkStatusLog)
                },
                onAppLifecycleChanged = { state ->
                    handleAppLifecycleChangedWithHandle(getHandle, state, onNetworkStatusLog)
                },
                onCleanupConnections = { reason ->
                    handleCleanupConnectionsWithHandle(getHandle, reason, onNetworkStatusLog)
                },
                onForceReconnect = { reason ->
                    handleForceReconnectWithHandle(getHandle, reason, onNetworkStatusLog)
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
                val result = handle.handleNetworkPathChangedCatching(snapshot)
                result
                    .onSuccess { eventResult ->
                        Log.i(
                            TAG,
                            "Network path changed event handled successfully: $eventResult",
                        )
                        onLog?.invoke(
                            "🌐 Network path changed - " +
                                "avail=${snapshot.availability}, " +
                                "wifi=${snapshot.transport.wifi}, " +
                                "cell=${snapshot.transport.cellular}, " +
                                "vpn=${snapshot.transport.vpn}",
                        )
                    }.onFailure { error ->
                        if (error is ActrException.Internal) {
                            Log.d(TAG, "Runtime node no longer available, skipping network path changed event")
                        } else {
                            Log.e(TAG, "Failed to handle network path changed event", error)
                            onLog?.invoke("❌ Network path changed event failed: ${error.message}")
                        }
                    }
            } catch (e: ActrException.Internal) {
                Log.d(TAG, "Runtime node no longer available, skipping network path changed event")
            } catch (e: Exception) {
                Log.e(TAG, "Error handling network path changed", e)
                onLog?.invoke("❌ Network path changed error: ${e.message}")
            }
        }

        private suspend fun handleAppLifecycleChangedInternal(
            getSystem: () -> ActrNode?,
            state: AppLifecycleState,
            onLog: ((String) -> Unit)?,
        ) {
            val system = getSystem()
            if (system == null) {
                Log.d(TAG, "ActrNode not available, skipping app lifecycle event")
                return
            }

            try {
                val handle = system.createNetworkEventHandle()
                val result = handle.handleAppLifecycleChangedCatching(state)
                result
                    .onSuccess { eventResult ->
                        Log.i(
                            TAG,
                            "App lifecycle event handled successfully: $eventResult",
                        )
                        val label = when (state) {
                            is AppLifecycleState.Background -> "Background"
                            is AppLifecycleState.Foreground ->
                                "Foreground (bg_duration=${state.backgroundDurationMs}ms)"
                        }
                        onLog?.invoke("📱 App lifecycle: $label")
                    }.onFailure { error ->
                        if (error is ActrException.Internal) {
                            Log.d(TAG, "Runtime node no longer available, skipping app lifecycle event")
                        } else {
                            Log.e(TAG, "Failed to handle app lifecycle event", error)
                            onLog?.invoke("❌ App lifecycle event failed: ${error.message}")
                        }
                    }
            } catch (e: ActrException.Internal) {
                Log.d(TAG, "Runtime node no longer available, skipping app lifecycle event")
            } catch (e: Exception) {
                Log.e(TAG, "Error handling app lifecycle", e)
                onLog?.invoke("❌ App lifecycle error: ${e.message}")
            }
        }

        private suspend fun handleCleanupConnectionsInternal(
            getSystem: () -> ActrNode?,
            reason: CleanupReason,
            onLog: ((String) -> Unit)?,
        ) {
            val system = getSystem()
            if (system == null) {
                Log.d(TAG, "ActrNode not available, skipping cleanup connections")
                return
            }

            try {
                val handle = system.createNetworkEventHandle()
                val result = handle.cleanupConnectionsCatching(reason)
                result
                    .onSuccess { eventResult ->
                        Log.i(TAG, "Cleanup connections handled successfully: $eventResult")
                        onLog?.invoke("🧹 Cleanup connections: $reason")
                    }.onFailure { error ->
                        if (error is ActrException.Internal) {
                            Log.d(TAG, "Runtime node no longer available, skipping cleanup connections")
                        } else {
                            Log.e(TAG, "Failed to cleanup connections", error)
                            onLog?.invoke("❌ Cleanup connections failed: ${error.message}")
                        }
                    }
            } catch (e: ActrException.Internal) {
                Log.d(TAG, "Runtime node no longer available, skipping cleanup connections")
            } catch (e: Exception) {
                Log.e(TAG, "Error cleaning up connections", e)
                onLog?.invoke("❌ Cleanup connections error: ${e.message}")
            }
        }

        private suspend fun handleForceReconnectInternal(
            getSystem: () -> ActrNode?,
            reason: ReconnectReason,
            onLog: ((String) -> Unit)?,
        ) {
            val system = getSystem()
            if (system == null) {
                Log.d(TAG, "ActrNode not available, skipping force reconnect")
                return
            }

            try {
                val handle = system.createNetworkEventHandle()
                val result = handle.forceReconnectCatching(reason)
                result
                    .onSuccess { eventResult ->
                        Log.i(TAG, "Force reconnect handled successfully: $eventResult")
                        onLog?.invoke("🔄 Force reconnect: $reason")
                    }.onFailure { error ->
                        if (error is ActrException.Internal) {
                            Log.d(TAG, "Runtime node no longer available, skipping force reconnect")
                        } else {
                            Log.e(TAG, "Failed to force reconnect", error)
                            onLog?.invoke("❌ Force reconnect failed: ${error.message}")
                        }
                    }
            } catch (e: ActrException.Internal) {
                Log.d(TAG, "Runtime node no longer available, skipping force reconnect")
            } catch (e: Exception) {
                Log.e(TAG, "Error force reconnecting", e)
                onLog?.invoke("❌ Force reconnect error: ${e.message}")
            }
        }

        // ---- Handle-based variants ----

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
                val result = handle.handleNetworkPathChangedCatching(snapshot)
                result
                    .onSuccess { eventResult ->
                        Log.i(
                            TAG,
                            "Network path changed event handled successfully: $eventResult",
                        )
                        onLog?.invoke(
                            "🌐 Network path changed - " +
                                "avail=${snapshot.availability}, " +
                                "wifi=${snapshot.transport.wifi}",
                        )
                    }.onFailure { error ->
                        if (error is ActrException.Internal) {
                            Log.d(TAG, "Runtime node no longer available, skipping network path changed event")
                        } else {
                            Log.e(TAG, "Failed to handle network path changed event", error)
                            onLog?.invoke("❌ Network path changed event failed: ${error.message}")
                        }
                    }
            } catch (e: Exception) {
                Log.e(TAG, "Error handling network path changed", e)
                onLog?.invoke("❌ Network path changed error: ${e.message}")
            }
        }

        private suspend fun handleAppLifecycleChangedWithHandle(
            getHandle: () -> NetworkEventHandle?,
            state: AppLifecycleState,
            onLog: ((String) -> Unit)?,
        ) {
            val handle = getHandle()
            if (handle == null) {
                Log.d(TAG, "NetworkEventHandle not available, skipping app lifecycle event")
                return
            }

            try {
                val result = handle.handleAppLifecycleChangedCatching(state)
                result
                    .onSuccess { eventResult ->
                        Log.i(TAG, "App lifecycle event handled successfully: $eventResult")
                        val label = when (state) {
                            is AppLifecycleState.Background -> "Background"
                            is AppLifecycleState.Foreground ->
                                "Foreground (bg_duration=${state.backgroundDurationMs}ms)"
                        }
                        onLog?.invoke("📱 App lifecycle: $label")
                    }.onFailure { error ->
                        if (error is ActrException.Internal) {
                            Log.d(TAG, "Runtime node no longer available, skipping app lifecycle event")
                        } else {
                            Log.e(TAG, "Failed to handle app lifecycle event", error)
                            onLog?.invoke("❌ App lifecycle event failed: ${error.message}")
                        }
                    }
            } catch (e: Exception) {
                Log.e(TAG, "Error handling app lifecycle", e)
                onLog?.invoke("❌ App lifecycle error: ${e.message}")
            }
        }

        private suspend fun handleCleanupConnectionsWithHandle(
            getHandle: () -> NetworkEventHandle?,
            reason: CleanupReason,
            onLog: ((String) -> Unit)?,
        ) {
            val handle = getHandle()
            if (handle == null) {
                Log.d(TAG, "NetworkEventHandle not available, skipping cleanup connections")
                return
            }

            try {
                val result = handle.cleanupConnectionsCatching(reason)
                result
                    .onSuccess { eventResult ->
                        Log.i(TAG, "Cleanup connections handled successfully: $eventResult")
                        onLog?.invoke("🧹 Cleanup connections: $reason")
                    }.onFailure { error ->
                        if (error is ActrException.Internal) {
                            Log.d(TAG, "Runtime node no longer available, skipping cleanup connections")
                        } else {
                            Log.e(TAG, "Failed to cleanup connections", error)
                            onLog?.invoke("❌ Cleanup connections failed: ${error.message}")
                        }
                    }
            } catch (e: Exception) {
                Log.e(TAG, "Error cleaning up connections", e)
                onLog?.invoke("❌ Cleanup connections error: ${e.message}")
            }
        }

        private suspend fun handleForceReconnectWithHandle(
            getHandle: () -> NetworkEventHandle?,
            reason: ReconnectReason,
            onLog: ((String) -> Unit)?,
        ) {
            val handle = getHandle()
            if (handle == null) {
                Log.d(TAG, "NetworkEventHandle not available, skipping force reconnect")
                return
            }

            try {
                val result = handle.forceReconnectCatching(reason)
                result
                    .onSuccess { eventResult ->
                        Log.i(TAG, "Force reconnect handled successfully: $eventResult")
                        onLog?.invoke("🔄 Force reconnect: $reason")
                    }.onFailure { error ->
                        if (error is ActrException.Internal) {
                            Log.d(TAG, "Runtime node no longer available, skipping force reconnect")
                        } else {
                            Log.e(TAG, "Failed to force reconnect", error)
                            onLog?.invoke("❌ Force reconnect failed: ${error.message}")
                        }
                    }
            } catch (e: Exception) {
                Log.e(TAG, "Error force reconnecting", e)
                onLog?.invoke("❌ Force reconnect error: ${e.message}")
            }
        }
    }

    private var connectivityManager: ConnectivityManager? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var isMonitoring = false
    private val eventAdapter = MobileEventAdapterState()
    private data class LifecycleDelivery(
        val state: AppLifecycleState,
        val reportNetworkSnapshotAfter: Boolean,
        val gate: MobileEventDeliveryGate,
    )

    private val lifecycleDeliveries = Channel<LifecycleDelivery>(Channel.UNLIMITED)
    @Volatile private var deliveryGate = MobileEventDeliveryGate()

    init {
        val deliveryJob =
            scope.launch(Dispatchers.IO) {
                for (delivery in lifecycleDeliveries) {
                    try {
                        delivery.gate.deliver {
                            onAppLifecycleChanged?.invoke(delivery.state)
                            if (delivery.reportNetworkSnapshotAfter) {
                                onNetworkPathChanged?.invoke(buildCurrentNetworkSnapshot())
                            }
                        }
                    } catch (e: Exception) {
                        Log.e(TAG, "Failed to deliver app lifecycle state: ${e.message}", e)
                    }
                }
            }
        deliveryJob.invokeOnCompletion { lifecycleDeliveries.close() }
    }

    // Current network state
    private var isNetworkAvailable = false
    private var isWifiConnected = false
    private var isCellularConnected = false
    private var isVpnConnected = false
    private var isEthernetConnected = false
    private var isExpensive = false
    private var isConstrained = false

    /** Start network monitoring */
    fun startMonitoring() {
        if (isMonitoring) {
            Log.d(TAG, "Network monitoring already running")
            return
        }

        try {
            deliveryGate = MobileEventDeliveryGate()
            connectivityManager =
                context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager

            setupNetworkCallback()

            isMonitoring = true
            Log.i(TAG, "Starting network state monitoring...")

            injectInitialLifecycleState()

            // Log initial network state
            logCurrentNetworkState("initial state")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to start network monitoring: ${e.message}", e)
        }
    }

    /** Stop network monitoring */
    fun stopMonitoring() {
        deliveryGate.close()
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

    // ---- App Lifecycle Methods ----

    @Synchronized
    private fun injectInitialLifecycleState() {
        val processInfo = ActivityManager.RunningAppProcessInfo()
        ActivityManager.getMyMemoryState(processInfo)
        val isForeground =
            processInfo.importance == ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
        val state =
            eventAdapter.initializePhase(isForeground, System.currentTimeMillis()) ?: return
        Log.i(TAG, "Initial app lifecycle state: ${if (isForeground) "foreground" else "background"}")
        dispatchLifecycleState(state, reportNetworkSnapshotAfter = isForeground)
    }

    private fun dispatchLifecycleState(
        state: AppLifecycleState,
        reportNetworkSnapshotAfter: Boolean = false,
    ) {
        val result =
            lifecycleDeliveries.trySend(
                LifecycleDelivery(state, reportNetworkSnapshotAfter, deliveryGate),
            )
        if (result.isFailure) {
            Log.w(TAG, "Lifecycle delivery skipped because its scope is no longer active")
        }
    }

    /**
     * Call this from Activity.onPause / onStop when the app goes to background.
     *
     * This records the background entry time and notifies the runtime.
     */
    @Synchronized
    fun onAppBackground() {
        val nowMs = System.currentTimeMillis()
        val state = eventAdapter.enterBackground(nowMs)
        Log.i(TAG, "App entered background at $nowMs")
        dispatchLifecycleState(state)
    }

    /**
     * Call this from Activity.onResume / onStart when the app returns to foreground.
     *
     * This computes the background duration, notifies the runtime, and reports
     * the current NetworkSnapshot.
     */
    @Synchronized
    fun onAppForeground() {
        val state = eventAdapter.enterForeground(System.currentTimeMillis())
        val backgroundDurationMs = (state as AppLifecycleState.Foreground).backgroundDurationMs

        Log.i(TAG, "App returned to foreground, background duration: ${backgroundDurationMs}ms")
        dispatchLifecycleState(state, reportNetworkSnapshotAfter = true)
    }

    /** Cleanup connections without reconnecting (e.g. app terminating, user logout). */
    fun cleanupConnections(reason: CleanupReason = CleanupReason.MANUAL_RESET) {
        val gate = deliveryGate
        scope.launch(Dispatchers.IO) {
            try {
                gate.deliver { onCleanupConnections?.invoke(reason) }
            } catch (e: Exception) {
                Log.e(TAG, "Failed to cleanup connections: ${e.message}", e)
            }
        }
    }

    /** Force cleanup and reconnect (e.g. manual reconnect, long background). */
    fun forceReconnect(reason: ReconnectReason = ReconnectReason.MANUAL_RECONNECT) {
        val gate = deliveryGate
        scope.launch(Dispatchers.IO) {
            try {
                gate.deliver { onForceReconnect?.invoke(reason) }
            } catch (e: Exception) {
                Log.e(TAG, "Failed to force reconnect: ${e.message}", e)
            }
        }
    }

    // ---- NetworkSnapshot Construction ----

    /** Build a NetworkSnapshot from the current network state. */
    private fun buildCurrentNetworkSnapshot(): NetworkSnapshot =
        eventAdapter.snapshot(
            isAvailable = isNetworkAvailable,
            transport =
                NetworkTransportFlags(
                    wifi = isWifiConnected,
                    cellular = isCellularConnected,
                    ethernet = isEthernetConnected,
                    vpn = isVpnConnected,
                    other = false,
                ),
            isExpensive = isExpensive,
            isConstrained = isConstrained,
        )

    /** Setup network callback */
    private fun setupNetworkCallback() {
        val callbackGate = deliveryGate
        val networkRequest =
            NetworkRequest
                .Builder()
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

                    val wasNetworkAvailable = isNetworkAvailable
                    val wasWifiConnected = isWifiConnected
                    val wasCellularConnected = isCellularConnected
                    val wasVpnConnected = isVpnConnected
                    val wasEthernetConnected = isEthernetConnected

                    updateNetworkState()

                    // Detect network path change
                    if (wasWifiConnected != isWifiConnected ||
                        wasCellularConnected != isCellularConnected ||
                        wasVpnConnected != isVpnConnected ||
                        wasEthernetConnected != isEthernetConnected ||
                        !wasNetworkAvailable && isNetworkAvailable
                    ) {
                        notifyNetworkPathChanged(
                            reason = "onAvailable (wasAvail=$wasNetworkAvailable nowAvail=$isNetworkAvailable)",
                            gate = callbackGate,
                        )
                    }

                    // Only notify pure availability transitions already handled above
                }

                override fun onLost(network: Network) {
                    super.onLost(network)
                    Log.w(TAG, "Network lost: $network")

                    val wasNetworkAvailable = isNetworkAvailable
                    updateNetworkState()

                    if (wasNetworkAvailable && !isNetworkAvailable) {
                        notifyNetworkPathChanged(reason = "onLost (avail→unavail)", gate = callbackGate)
                    } else if (wasNetworkAvailable) {
                        // Still have other networks, but path may have changed
                        notifyNetworkPathChanged(reason = "onLost partial", gate = callbackGate)
                    }
                }

                override fun onCapabilitiesChanged(
                    network: Network,
                    networkCapabilities: NetworkCapabilities,
                ) {
                    super.onCapabilitiesChanged(network, networkCapabilities)

                    val wasWifiConnected = isWifiConnected
                    val wasCellularConnected = isCellularConnected
                    val wasVpnConnected = isVpnConnected
                    val wasEthernetConnected = isEthernetConnected
                    val wasExpensive = isExpensive
                    val wasConstrained = isConstrained

                    readCapabilities(networkCapabilities)

                    Log.d(
                        TAG,
                        "Network capability changed - " +
                            "WiFi: $isWifiConnected, Cellular: $isCellularConnected, " +
                            "VPN: $isVpnConnected, Ethernet: $isEthernetConnected, " +
                            "Expensive: $isExpensive, Constrained: $isConstrained",
                    )

                    if (wasWifiConnected != isWifiConnected ||
                        wasCellularConnected != isCellularConnected ||
                        wasVpnConnected != isVpnConnected ||
                        wasEthernetConnected != isEthernetConnected ||
                        wasExpensive != isExpensive ||
                        wasConstrained != isConstrained
                    ) {
                        val networkType = getNetworkTypeLabel()
                        Log.i(
                            TAG,
                            "Network type/capability changed: $networkType",
                        )

                        notifyNetworkPathChanged(
                            reason = "capabilitiesChanged → $networkType",
                            gate = callbackGate,
                        )
                    }
                }

                override fun onLinkPropertiesChanged(
                    network: Network,
                    linkProperties: android.net.LinkProperties,
                ) {
                    super.onLinkPropertiesChanged(network, linkProperties)
                    Log.d(TAG, "Network link properties changed: $network")
                }
            }

        connectivityManager?.registerNetworkCallback(networkRequest, networkCallback!!)
    }

    /** Notify the upper layer about a network path change via the unified callback. */
    private fun notifyNetworkPathChanged(
        reason: String,
        gate: MobileEventDeliveryGate = deliveryGate,
    ) {
        val snapshot = buildCurrentNetworkSnapshot()
        Log.i(
            TAG,
            "Network path changed ($reason): seq=${snapshot.sequence}, " +
                "avail=${snapshot.availability}, " +
                "wifi=${snapshot.transport.wifi}, cell=${snapshot.transport.cellular}, " +
                "eth=${snapshot.transport.ethernet}, vpn=${snapshot.transport.vpn}, " +
                "expensive=${snapshot.isExpensive}, constrained=${snapshot.isConstrained}",
        )

        scope.launch(Dispatchers.IO) {
            try {
                gate.deliver { onNetworkPathChanged?.invoke(snapshot) }
            } catch (e: Exception) {
                Log.e(TAG, "Failed to handle network path change: ${e.message}", e)
            }
        }
    }

    /** Update network state from current active network */
    private fun updateNetworkState() {
        val activeNetwork = connectivityManager?.activeNetwork
        val capabilities =
            activeNetwork?.let { connectivityManager?.getNetworkCapabilities(it) }

        val wasNetworkAvailable = isNetworkAvailable
        isNetworkAvailable = activeNetwork != null && capabilities != null

        if (capabilities != null) {
            readCapabilities(capabilities)
        } else {
            isWifiConnected = false
            isCellularConnected = false
            isEthernetConnected = false
            isVpnConnected = false
            isExpensive = false
            isConstrained = false
        }

        val availabilityChange =
            if (wasNetworkAvailable != isNetworkAvailable) {
                if (isNetworkAvailable) "became available" else "became unavailable"
            } else {
                ""
            }

        Log.d(
            TAG,
            "Network state updated - Available: $isNetworkAvailable $availabilityChange, " +
                "WiFi: $isWifiConnected, Cellular: $isCellularConnected, " +
                "Ethernet: $isEthernetConnected, VPN: $isVpnConnected",
        )
    }

    /** Read transport and capability flags from NetworkCapabilities. */
    private fun readCapabilities(capabilities: NetworkCapabilities) {
        isWifiConnected = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
        isCellularConnected = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
        isEthernetConnected = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
        isVpnConnected = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)

        // Metered/expensive: NET_CAPABILITY_NOT_METERED means NOT expensive
        isExpensive =
            !capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)

        // Constrained: NET_CAPABILITY_NOT_CONGESTED means NOT constrained (inverse)
        isConstrained =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                !capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_CONGESTED)
            } else {
                false
            }
    }

    /** Get a human-readable network type label. */
    private fun getNetworkTypeLabel(): String =
        when {
            isVpnConnected -> "VPN"
            isWifiConnected -> "WiFi"
            isCellularConnected -> "Cellular"
            isEthernetConnected -> "Ethernet"
            else -> "Unknown"
        }

    /** Log current network state */
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

                if (transports.isNotEmpty()) transports.joinToString(", ") else "no transport types"
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
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) == true -> "Cellular"
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) == true -> "Ethernet"
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
        updateNetworkState()
        logCurrentNetworkState("manual check")

        notifyNetworkPathChanged(reason = "manual check")
    }

    /** Check if currently have network connection */
    fun isConnected(): Boolean = isNetworkAvailable

    /** Check if currently connected via WiFi */
    fun isWifi(): Boolean = isWifiConnected

    /** Check if currently connected via mobile network */
    fun isCellular(): Boolean = isCellularConnected

    /** Check if currently connected via VPN */
    fun isVpn(): Boolean = isVpnConnected

    /** Check if currently connected via Ethernet */
    fun isEthernet(): Boolean = isEthernetConnected

    /** Check if current network is metered/expensive */
    fun isNetworkExpensive(): Boolean = isExpensive

    /** Check if current network is constrained/congested */
    fun isNetworkConstrained(): Boolean = isConstrained
}

// ============================================================================
// ActrNode Integration — one-shot monitor setup
// ============================================================================

/**
 * Create and start a [NetworkMonitor] wired to this [ActrNode].
 *
 * The returned monitor is already started. The node reference is captured
 * immediately, so you don't need a lazy lambda.
 *
 * Example:
 * ```kotlin
 * val system = ActrNode.fromPackageFile("config.toml", "dist/app.actr")
 * val monitor = system.createNetworkMonitor(this, lifecycleScope) { Log.d("App", it) }
 * ```
 *
 * @param context Android Context (for ConnectivityManager)
 * @param scope CoroutineScope (typically lifecycleScope)
 * @param onNetworkStatusLog Optional callback for network status messages
 * @return A started [NetworkMonitor] instance
 */
fun ActrNode.createNetworkMonitor(
    context: android.content.Context,
    scope: kotlinx.coroutines.CoroutineScope,
    onNetworkStatusLog: ((String) -> Unit)? = null,
): NetworkMonitor {
    val monitor = NetworkMonitor.create(
        context = context,
        scope = scope,
        getSystem = { this },
        onNetworkStatusLog = onNetworkStatusLog,
    )
    monitor.startMonitoring()
    return monitor
}
