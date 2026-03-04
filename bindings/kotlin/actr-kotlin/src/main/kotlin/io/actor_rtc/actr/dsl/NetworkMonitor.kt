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
 * NetworkMonitor - 独立的网络状态监控器
 *
 * 功能特性：
 * - 监控WiFi、移动网络、以太网和VPN连接状态变化
 * - 详细记录所有网络事件到日志，包括网络可用/丢失和类型变化
 * - 支持网络类型变化回调通知（WiFi/Cellular/VPN切换）
 * - 支持网络连接状态变化回调（可用/不可用）
 * - 自动检测初始网络状态
 * - 支持手动触发网络状态检查
 *
 * 回调说明：
 * - onNetworkTypeChanged: 网络类型变化时调用（WiFi/移动网络/VPN切换）
 * - onNetworkAvailable: 网络从不可用变为可用时调用
 * - onNetworkLost: 网络从可用变为不可用时调用
 *
 * 日志输出示例：
 * - 网络可用/丢失事件（真正的连接状态变化）
 * - 网络能力变化（WiFi/移动网络/VPN状态）
 * - 网络类型切换事件
 * - 当前网络状态摘要
 *
 * 使用方法：
 * 1. 创建实例：NetworkMonitor(context, scope, onNetworkTypeChanged, onNetworkAvailable, onNetworkLost)
 * 2. 启动监控：startMonitoring()
 * 3. 停止监控：stopMonitoring() （通常在Activity.onDestroy中调用）
 * 4. 手动检查：triggerNetworkCheck()
 * 5. 获取状态：getCurrentNetworkStatus()
 *
 * 与 ActrSystem 集成：
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
         * 创建一个与 ActrSystem 集成的 NetworkMonitor
         *
         * 此工厂方法自动将网络事件转发给 ActrSystem 的 NetworkEventHandle， 使用户无需手动处理网络事件。
         *
         * @param context Android Context
         * @param scope CoroutineScope，通常使用 lifecycleScope
         * @param getSystem 获取 ActrSystem 实例的函数（可能返回 null，例如系统尚未初始化时）
         * @param onNetworkStatusLog 可选的日志回调，用于显示网络状态变化
         * @return NetworkMonitor 实例
         *
         * Example:
         * ```kotlin
         * var system: ActrSystem? = null
         * val monitor = NetworkMonitor.create(this, lifecycleScope, { system }) { msg ->
         *     Log.d("App", msg)
         * }
         * monitor.startMonitoring()
         *
         * // 稍后初始化 system
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
         * 创建一个与 NetworkEventHandle 集成的 NetworkMonitor
         *
         * 此工厂方法自动将网络事件转发给指定的 NetworkEventHandle。
         *
         * @param context Android Context
         * @param scope CoroutineScope，通常使用 lifecycleScope
         * @param getHandle 获取 NetworkEventHandle 实例的函数（可能返回 null）
         * @param onNetworkStatusLog 可选的日志回调，用于显示网络状态变化
         * @return NetworkMonitor 实例
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

    // 当前网络状态
    private var isNetworkAvailable = false
    private var isWifiConnected = false
    private var isCellularConnected = false
    private var isVpnConnected = false

    /** 开始网络监控 */
    fun startMonitoring() {
        if (isMonitoring) {
            Log.d(TAG, "网络监控已在运行中")
            return
        }

        try {
            connectivityManager =
                    context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager

            setupNetworkCallback()

            isMonitoring = true
            Log.i(TAG, "开始网络状态监控...")

            // 记录初始网络状态
            logCurrentNetworkState("初始状态")
        } catch (e: Exception) {
            Log.e(TAG, "启动网络监控失败: ${e.message}", e)
        }
    }

    /** 停止网络监控 */
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
            Log.i(TAG, "停止网络监控")
        } catch (e: Exception) {
            Log.e(TAG, "停止网络监控失败: ${e.message}", e)
        }
    }

    /** 设置网络回调 */
    private fun setupNetworkCallback() {
        val networkRequest =
                NetworkRequest.Builder()
                        .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                        // 移除阻止VPN回调的能力
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
                        Log.i(TAG, "网络可用: $network")

                        // 保存之前的网络类型状态
                        val wasNetworkAvailable = isNetworkAvailable
                        val wasWifiConnected = isWifiConnected
                        val wasCellularConnected = isCellularConnected
                        val wasVpnConnected = isVpnConnected

                        updateNetworkState()

                        // 检查网络类型是否发生变化（在网络可用事件中也需要检测）
                        if (wasWifiConnected != isWifiConnected ||
                                        wasCellularConnected != isCellularConnected ||
                                        wasVpnConnected != isVpnConnected
                        ) {
                            val networkType =
                                    when {
                                        isVpnConnected -> "VPN"
                                        isWifiConnected -> "WiFi"
                                        isCellularConnected -> "移动网络"
                                        else -> "未知"
                                    }

                            Log.i(
                                    TAG,
                                    "网络类型变化 (onAvailable): $networkType (WiFi: $isWifiConnected, Cellular: $isCellularConnected, VPN: $isVpnConnected)"
                            )

                            // 通知监听器 - 使用 Dispatchers.IO 避免阻塞主线程
                            // 因为网络事件处理可能需要等待 WebSocket 重连等耗时操作
                            scope.launch(Dispatchers.IO) {
                                try {
                                    onNetworkTypeChanged(
                                            isWifiConnected,
                                            isCellularConnected,
                                            isVpnConnected
                                    )
                                } catch (e: Exception) {
                                    Log.e(TAG, "网络类型变化处理失败: ${e.message}", e)
                                }
                            }
                        }

                        // 只有在网络从不可用变为可用时才触发网络可用事件
                        if (!wasNetworkAvailable && isNetworkAvailable) {
                            Log.i(TAG, "网络连接状态变化: 不可用 -> 可用")
                            // 使用 Dispatchers.IO 避免阻塞主线程
                            scope.launch(Dispatchers.IO) { onNetworkAvailable?.invoke() }
                        }
                    }

                    override fun onLost(network: Network) {
                        super.onLost(network)
                        Log.w(TAG, "网络丢失: $network")

                        // 检查是否是真正的网络连接状态变化
                        val wasNetworkAvailable = isNetworkAvailable
                        updateNetworkState()

                        // 只有在网络从可用变为不可用时才触发网络丢失事件
                        if (wasNetworkAvailable && !isNetworkAvailable) {
                            Log.w(TAG, "网络连接状态变化: 可用 -> 不可用")
                            // 使用 Dispatchers.IO 避免阻塞主线程
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
                                "网络能力变化 - WiFi: $isWifiConnected, Cellular: $isCellularConnected, VPN: $isVpnConnected"
                        )
                        Log.d(TAG, "网络能力详情: $networkCapabilities")

                        // 检查网络类型是否发生变化
                        if (wasWifiConnected != isWifiConnected ||
                                        wasCellularConnected != isCellularConnected ||
                                        wasVpnConnected != isVpnConnected
                        ) {
                            val networkType =
                                    when {
                                        isVpnConnected -> "VPN"
                                        isWifiConnected -> "WiFi"
                                        isCellularConnected -> "移动网络"
                                        else -> "未知"
                                    }

                            Log.i(
                                    TAG,
                                    "网络类型变化: $networkType (WiFi: $isWifiConnected, Cellular: $isCellularConnected, VPN: $isVpnConnected)"
                            )

                            // 通知监听器 - 使用 Dispatchers.IO 避免阻塞主线程
                            // 因为网络事件处理可能需要等待 WebSocket 重连等耗时操作
                            scope.launch(Dispatchers.IO) {
                                try {
                                    onNetworkTypeChanged(
                                            isWifiConnected,
                                            isCellularConnected,
                                            isVpnConnected
                                    )
                                } catch (e: Exception) {
                                    Log.e(TAG, "网络类型变化处理失败: ${e.message}", e)
                                }
                            }
                        }
                    }

                    override fun onLinkPropertiesChanged(
                            network: Network,
                            linkProperties: android.net.LinkProperties
                    ) {
                        super.onLinkPropertiesChanged(network, linkProperties)
                        Log.d(TAG, "网络链接属性变化: $network")
                    }
                }

        connectivityManager?.registerNetworkCallback(networkRequest, networkCallback!!)
    }

    /** 更新网络状态 */
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
                    if (isNetworkAvailable) "变为可用" else "变为不可用"
                } else ""

        Log.d(
                TAG,
                "网络状态更新 - 可用: $isNetworkAvailable $availabilityChange, WiFi: $isWifiConnected, Cellular: $isCellularConnected, VPN: $isVpnConnected"
        )
    }

    /** 记录当前网络状态 */
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

                    if (transports.isNotEmpty()) transports.joinToString(", ") else "无传输类型"
                } else {
                    "无网络能力"
                }

        val contextStr = if (context.isNotEmpty()) " ($context)" else ""
        Log.i(TAG, "当前网络状态$contextStr: $networkInfo")
    }

    /** 获取当前网络状态摘要 */
    fun getCurrentNetworkStatus(): String {
        return try {
            val activeNetwork = connectivityManager?.activeNetwork
            val capabilities =
                    activeNetwork?.let { connectivityManager?.getNetworkCapabilities(it) }

            when {
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true -> "VPN"
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true -> "WiFi"
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) == true -> "移动网络"
                capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) == true -> "以太网"
                activeNetwork != null -> "网络连接 (未知类型)"
                else -> "无网络连接"
            }
        } catch (e: Exception) {
            Log.e(TAG, "获取网络状态失败: ${e.message}", e)
            "获取状态失败"
        }
    }

    /** 手动触发网络状态检查 */
    fun triggerNetworkCheck() {
        Log.i(TAG, "手动触发网络状态检查")
        updateNetworkState()
        logCurrentNetworkState("手动检查")
    }

    /** 当前是否有网络连接 */
    fun isConnected(): Boolean = isNetworkAvailable

    /** 当前是否通过 WiFi 连接 */
    fun isWifi(): Boolean = isWifiConnected

    /** 当前是否通过移动网络连接 */
    fun isCellular(): Boolean = isCellularConnected

    /** 当前是否通过 VPN 连接 */
    fun isVpn(): Boolean = isVpnConnected
}
