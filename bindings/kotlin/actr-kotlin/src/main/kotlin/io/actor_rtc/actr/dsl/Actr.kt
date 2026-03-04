/**
 * Actor-RTC Kotlin SDK
 *
 * A Kotlin-idiomatic wrapper for the Actor-RTC framework.
 *
 * Example usage:
 * ```kotlin
 * // Create and start an actor
 * val system = ActrSystem.fromFile("config.toml")
 * val ref = system.attach(myWorkload).start()
 *
 * // Discover and call remote services
 * val echoService = ref.discover("acme:EchoService").firstOrNull()
 * val response = ref.call(echoService, "echo.EchoService.Echo", request)
 *
 * // Send data stream
 * ref.sendStream(target) {
 *     streamId = "stream-001"
 *     sequence = 0uL
 *     payload = data
 *     metadata {
 *         "content-type" to "application/octet-stream"
 *     }
 * }
 *
 * // Clean shutdown
 * ref.shutdown()
 * ref.awaitShutdown()
 * ```
 */
package io.actor_rtc.actr.dsl

import io.actor_rtc.actr.ActrException
import io.actor_rtc.actr.ActrId
import io.actor_rtc.actr.ActrNodeWrapper
import io.actor_rtc.actr.ActrRefWrapper
import io.actor_rtc.actr.ActrSystemWrapper
import io.actor_rtc.actr.ActrType
import io.actor_rtc.actr.NetworkEventHandleWrapper
import io.actor_rtc.actr.WorkloadBridge

// ============================================================================
// Type Aliases - Provide cleaner names without "Wrapper" suffix
// ============================================================================

/** Entry point for creating actors. Use [ActrSystem.fromFile] to create an instance. */
typealias ActrSystem = ActrSystemWrapper

/** A prepared actor node ready to be started. Call [start] to get an [ActrRef]. */
typealias ActrNode = ActrNodeWrapper

/**
 * Reference to a running actor. Provides methods for:
 * - [ActrRef.call]
 * - RPC calls to remote actors
 * - [ActrRef.discover]
 * - Service discovery
 * - [ActrRef.sendDataStream]
 * - Send data streams
 * - [ActrRef.shutdown]
 * - Graceful shutdown
 */
typealias ActrRef = ActrRefWrapper

/** Handle for network event callbacks. Used for platform integration. */
typealias NetworkEventHandle = NetworkEventHandleWrapper

/** Workload callback interface for handling lifecycle events. */
typealias Workload = WorkloadBridge

// ============================================================================
// ActrSystem Factory Functions
// ============================================================================

/**
 * Create an ActrSystem from a configuration file.
 *
 * Example:
 * ```kotlin
 * val system = ActrSystem.fromFile("config.toml")
 * ```
 *
 * @param configPath Path to the TOML configuration file
 * @return A new ActrSystem instance
 * @throws ActrException.ConfigException if the config file is invalid
 */
suspend fun ActrSystemWrapper.Companion.fromFile(configPath: String): ActrSystem {
    return ActrSystemWrapper.newFromFile(configPath)
}

/**
 * Create an ActrSystem from a configuration file (top-level function).
 *
 * Example:
 * ```kotlin
 * val system = createActrSystem("config.toml")
 * ```
 *
 * @param configPath Path to the TOML configuration file
 * @return A new ActrSystem instance
 * @throws ActrException.ConfigException if the config file is invalid
 */
suspend fun createActrSystem(configPath: String): ActrSystem {
    return ActrSystemWrapper.newFromFile(configPath)
}

// ============================================================================
// ActrSystem Extensions
// ============================================================================

/**
 * Create a network event handle for platform callbacks.
 *
 * This handle is used to notify the actor system about network state changes,
 * which is important for WebRTC connection management on mobile platforms.
 *
 * Example:
 * ```kotlin
 * val system = createActrSystem("config.toml")
 * val networkHandle = system.createNetworkEventHandle()
 *
 * // Notify when network becomes available
 * networkHandle.handleNetworkAvailable()
 * ```
 *
 * @return A new NetworkEventHandle instance
 * @throws ActrException if the handle cannot be created
 */
suspend fun ActrSystem.createNetworkEventHandle(): NetworkEventHandle {
    return createNetworkEventHandle()
}

// ============================================================================
// ActrRef Extensions
// ============================================================================

/**
 * Discover actors of the specified type using a type string.
 *
 * @param typeString Actor type in "manufacturer:name" format (e.g., "acme:EchoService")
 * @param count Maximum number of candidates to return (default: 1)
 * @return List of discovered actor IDs
 */
suspend fun ActrRef.discover(typeString: String, count: UInt = 1u): List<ActrId> {
    return discover(typeString.toActrType(), count)
}

/**
 * Discover a single actor of the specified type.
 *
 * @param typeString Actor type in "manufacturer:name" format
 * @return The first discovered actor ID, or null if none found
 */
suspend fun ActrRef.discoverOne(typeString: String): ActrId? {
    return discover(typeString, 1u).firstOrNull()
}

/**
 * Discover a single actor of the specified type.
 *
 * @param type Actor type
 * @return The first discovered actor ID, or null if none found
 */
suspend fun ActrRef.discoverOne(type: ActrType): ActrId? {
    return discover(type, 1u).firstOrNull()
}

/**
 * Send a DataStream built with DSL syntax.
 *
 * Example:
 * ```kotlin
 * workload.sendStream(targetId) {
 *     streamId = "my-stream"
 *     sequence = 0uL
 *     payload = "Hello".toByteArray()
 *     metadata {
 *         "key1" to "value1"
 *         "key2" to "value2"
 *     }
 * }
 * ```
 */
suspend fun SimpleWorkload.sendStream(target: ActrId, builder: DataStreamBuilder.() -> Unit) {
    val dataStream = DataStreamBuilder().apply(builder).build()
    sendDataStream(target, dataStream)
}

/** Await shutdown completion. Alias for [waitForShutdown]. */
suspend fun ActrRef.awaitShutdown() {
    waitForShutdown()
}

/** Check if this actor reference is still valid (not destroyed). */
val ActrRef.isActive: Boolean
    get() = !isShuttingDown()
