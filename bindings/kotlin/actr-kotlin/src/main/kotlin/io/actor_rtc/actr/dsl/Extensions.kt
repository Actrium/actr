/** Utility functions and extensions for Actor-RTC SDK. */
package io.actor_rtc.actr.dsl

import io.actor_rtc.actr.ActrException
import io.actor_rtc.actr.ActrId
import io.actor_rtc.actr.ErrorKind
import io.actor_rtc.actr.NetworkEventResult
import io.actor_rtc.actr.PayloadType
import io.actor_rtc.actr.actrErrorIsRetryable
import io.actor_rtc.actr.actrErrorKind
import io.actor_rtc.actr.actrErrorRequiresDlq

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
//
// The underlying sealed `ActrException` mirrors `actr_protocol::ActrError`
// 1:1 (10 variants) plus a small number of binding-local variants. Rather
// than reasoning about each concrete subclass, consumers typically branch
// on fault domain via `ErrorKind` — see `actrErrorKind(ex)` below.

/** Get a user-friendly error message for logs or UI. */
val ActrException.userMessage: String
    get() =
            when (this) {
                is ActrException.Unavailable -> "Peer unavailable: $msg"
                is ActrException.TimedOut -> "Request timed out"
                is ActrException.NotFound -> "Not found: $msg"
                is ActrException.PermissionDenied -> "Permission denied: $msg"
                is ActrException.InvalidArgument -> "Invalid argument: $msg"
                is ActrException.UnknownRoute -> "Unknown route: $msg"
                is ActrException.DependencyNotFound ->
                        "Dependency '$serviceName' not found: $message"
                is ActrException.DecodeFailure -> "Decode failure: $msg"
                is ActrException.NotImplemented -> "Not implemented: $msg"
                is ActrException.Internal -> "Internal error: $msg"
                is ActrException.Config -> "Configuration error: $msg"
            }

/** Check if the exception is a timeout. */
val ActrException.isTimeout: Boolean
    get() = this is ActrException.TimedOut

/**
 * Check if the exception is a transient connectivity error — use this as a
 * hint for retrying with backoff.
 *
 * Prefer [isRecoverable] (which consults the fault-domain classification)
 * for new code.
 */
val ActrException.isConnectionError: Boolean
    get() = this is ActrException.Unavailable

/**
 * Check if the exception is recoverable (worth retrying).
 *
 * Delegates to the fault-domain classifier exported by the Rust binding:
 * only `ErrorKind.TRANSIENT` errors are retryable, everything else is a
 * terminal failure.
 */
val ActrException.isRecoverable: Boolean
    get() = actrErrorIsRetryable(this)

/**
 * Fault-domain bucket for this exception — one of `Transient` / `Client` /
 * `Internal` / `Corrupt`.
 */
val ActrException.kind: ErrorKind
    get() = actrErrorKind(this)

/**
 * `true` iff the underlying payload should be routed to a Dead Letter
 * Queue (only `ErrorKind.Corrupt` errors).
 */
val ActrException.requiresDlq: Boolean
    get() = actrErrorRequiresDlq(this)

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
 * node.withStartedActor { ref ->
 *     val target = ref.discoverOne("acme:EchoService")
 *     ref.call("echo.EchoService.Echo", payload)
 * }
 * // Actor is automatically shut down after the block
 * ```
 */
suspend fun <T> ActrNode.withStartedActor(block: suspend (ActrRef) -> T): T {
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
