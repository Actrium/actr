/** Utility functions and extensions for Actor-RTC SDK. */
package io.actor_rtc.actr.dsl

import io.actor_rtc.actr.ActrException
import io.actor_rtc.actr.ActrId
import io.actor_rtc.actr.NetworkEventResult
import io.actor_rtc.actr.PayloadType

// ============================================================================
// ActrRef Call Extensions - Convenience wrappers with default parameters
// ============================================================================

/**
 * Call via RPC proxy with default PayloadType.RPC_RELIABLE and 30s timeout.
 *
 * This sends a request through the local workload's RPC proxy mechanism.
 * The workload's dispatch() method handles routing to the remote actor.
 *
 * Example:
 * ```kotlin
 * val response = ref.call("echo.EchoService.Echo", requestPayload)
 * ```
 */
suspend fun ActrRef.call(
        routeKey: String,
        requestPayload: ByteArray,
        payloadType: PayloadType = PayloadType.RPC_RELIABLE,
        timeoutMs: Long = 30000L
): ByteArray {
    return call(routeKey, payloadType, requestPayload, timeoutMs)
}

/**
 * Send a one-way message via RPC proxy with default PayloadType.RPC_RELIABLE.
 *
 * This sends a message through the local workload's RPC proxy mechanism.
 * The workload's dispatch() method handles routing to the remote actor.
 *
 * Example:
 * ```kotlin
 * ref.tell("echo.EchoService.Notify", messagePayload)
 * ```
 */
suspend fun ActrRef.tell(
        routeKey: String,
        messagePayload: ByteArray,
        payloadType: PayloadType = PayloadType.RPC_RELIABLE
) {
    tell(routeKey, payloadType, messagePayload)
}

// ============================================================================
// Result Extensions - For functional error handling
// ============================================================================

/**
 * Execute an RPC call and wrap the result.
 *
 * Example:
 * ```kotlin
 * val result = ref.callCatching("echo.EchoService.Echo", payload)
 * result.onSuccess { response ->
 *     println("Got response: $response")
 * }.onFailure { error ->
 *     println("Call failed: $error")
 * }
 * ```
 */
suspend fun ActrRef.callCatching(
        routeKey: String,
        requestPayload: ByteArray,
        payloadType: PayloadType = PayloadType.RPC_RELIABLE,
        timeoutMs: Long = 30000L
): Result<ByteArray> {
    return runCatching { call(routeKey, requestPayload, payloadType, timeoutMs) }
}

/** Discover actors and wrap the result. */
suspend fun ActrRef.discoverCatching(typeString: String, count: UInt = 1u): Result<List<ActrId>> {
    return runCatching { discover(typeString, count) }
}

// ============================================================================
// NetworkEventHandle Extensions - For functional error handling
// ============================================================================

/**
 * Handle network available event and wrap the result.
 *
 * Example:
 * ```kotlin
 * val result = networkHandle.handleNetworkAvailableCatching()
 * result.onSuccess { eventResult ->
 *     println("Network available handled: $eventResult")
 * }.onFailure { error ->
 *     println("Failed to handle network available: $error")
 * }
 * ```
 */
suspend fun NetworkEventHandle.handleNetworkAvailableCatching(): Result<NetworkEventResult> {
    return runCatching { handleNetworkAvailable() }
}

/**
 * Handle network lost event and wrap the result.
 *
 * Example:
 * ```kotlin
 * val result = networkHandle.handleNetworkLostCatching()
 * result.onSuccess { eventResult ->
 *     println("Network lost handled: $eventResult")
 * }.onFailure { error ->
 *     println("Failed to handle network lost: $error")
 * }
 * ```
 */
suspend fun NetworkEventHandle.handleNetworkLostCatching(): Result<NetworkEventResult> {
    return runCatching { handleNetworkLost() }
}

/**
 * Handle network type changed event and wrap the result.
 *
 * Example:
 * ```kotlin
 * val result = networkHandle.handleNetworkTypeChangedCatching(true, false)
 * result.onSuccess { eventResult ->
 *     println("Network type changed handled: $eventResult")
 * }.onFailure { error ->
 *     println("Failed to handle network type changed: $error")
 * }
 * ```
 */
suspend fun NetworkEventHandle.handleNetworkTypeChangedCatching(
    isWifi: Boolean,
    isCellular: Boolean
): Result<NetworkEventResult> {
    return runCatching { handleNetworkTypeChanged(isWifi, isCellular) }
}

// ============================================================================
// Exception Extensions
// ============================================================================

/** Get a user-friendly error message. */
val ActrException.userMessage: String
    get() =
            when (this) {
                is ActrException.ConfigException -> "Configuration error: $msg"
                is ActrException.ConnectionException -> "Connection error: $msg"
                is ActrException.RpcException -> "RPC error: $msg"
                is ActrException.StateException -> "State error: $msg"
                is ActrException.InternalException -> "Internal error: $msg"
                is ActrException.TimeoutException -> "Timeout: $msg"
                is ActrException.WorkloadException -> "Workload error: $msg"
            }

/** Check if the exception is a timeout. */
val ActrException.isTimeout: Boolean
    get() = this is ActrException.TimeoutException

/** Check if the exception is a connection error. */
val ActrException.isConnectionError: Boolean
    get() = this is ActrException.ConnectionException

/** Check if the exception is recoverable (worth retrying). */
val ActrException.isRecoverable: Boolean
    get() =
            when (this) {
                is ActrException.TimeoutException -> true
                is ActrException.ConnectionException -> true
                is ActrException.RpcException -> false
                is ActrException.ConfigException -> false
                is ActrException.StateException -> false
                is ActrException.InternalException -> false
                is ActrException.WorkloadException -> false
            }

// ============================================================================
// Retry Utilities
// ============================================================================

/** Retry configuration for operations. */
data class RetryConfig(
        val maxAttempts: Int = 3,
        val initialDelayMs: Long = 1000,
        val maxDelayMs: Long = 10000,
        val factor: Double = 2.0
)

/**
 * Execute a suspending block with exponential backoff retry.
 *
 * Example:
 * ```kotlin
 * val result = withRetry(maxAttempts = 5) {
 *     ref.discover("acme:EchoService")
 * }
 * ```
 */
suspend fun <T> withRetry(
        maxAttempts: Int = 3,
        initialDelayMs: Long = 1000,
        maxDelayMs: Long = 10000,
        factor: Double = 2.0,
        shouldRetry: (Exception) -> Boolean = { it is ActrException && it.isRecoverable },
        block: suspend () -> T
): T {
    var currentDelay = initialDelayMs
    var lastException: Exception? = null

    repeat(maxAttempts) { attempt ->
        try {
            return block()
        } catch (e: Exception) {
            lastException = e
            if (attempt == maxAttempts - 1 || !shouldRetry(e)) {
                throw e
            }
            kotlinx.coroutines.delay(currentDelay)
            currentDelay = (currentDelay * factor).toLong().coerceAtMost(maxDelayMs)
        }
    }

    throw lastException ?: IllegalStateException("Retry failed without exception")
}

/** Execute a suspending block with retry using RetryConfig. */
suspend fun <T> withRetry(
        config: RetryConfig,
        shouldRetry: (Exception) -> Boolean = { it is ActrException && it.isRecoverable },
        block: suspend () -> T
): T =
        withRetry(
                maxAttempts = config.maxAttempts,
                initialDelayMs = config.initialDelayMs,
                maxDelayMs = config.maxDelayMs,
                factor = config.factor,
                shouldRetry = shouldRetry,
                block = block
        )

// ============================================================================
// Scoped Resource Management
// ============================================================================

/**
 * Execute a block with a started package-backed actor, ensuring proper cleanup.
 *
 * Example:
 * ```kotlin
 * system.withStartedActor { ref ->
 *     val target = ref.discoverOne("acme:EchoService")
 *     ref.call("echo.EchoService.Echo", payload)
 * }
 * // Actor is automatically shut down after the block
 * ```
 */
suspend fun <T> ActrSystem.withStartedActor(block: suspend (ActrRef) -> T): T {
    val ref = start()
    return try {
        block(ref)
    } finally {
        try {
            ref.shutdown()
            ref.awaitShutdown()
        } catch (_: Exception) {
            // Ignore cleanup errors
        }
    }
}
